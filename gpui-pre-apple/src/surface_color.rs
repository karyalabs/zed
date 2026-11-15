//! How a video surface's pixels become output colour.
//!
//! GPUI's first surface format, full-range 8-bit NV12, keeps its original
//! fixed BT.601 conversion with no colour management. Every other format is
//! read the way its CoreVideo attachments describe it: matrix, primaries
//! and transfer function, with PQ and HLG mapped to the brightness the
//! output can show (`headroom`, in units of SDR white).

use anyhow::{Result, bail};
use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetPixelFormatType, kCVImageBufferColorPrimaries_DCI_P3,
    kCVImageBufferColorPrimaries_ITU_R_2020, kCVImageBufferColorPrimaries_P3_D65,
    kCVImageBufferColorPrimariesKey, kCVImageBufferTransferFunction_ITU_R_2100_HLG,
    kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ, kCVImageBufferTransferFunctionKey,
    kCVImageBufferYCbCrMatrix_ITU_R_2020, kCVImageBufferYCbCrMatrix_ITU_R_601_4,
    kCVImageBufferYCbCrMatrix_SMPTE_240M_1995, kCVImageBufferYCbCrMatrixKey,
    kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange,
};
use objc2_metal::MTLPixelFormat;
use std::ptr;

/// The colour description the surface fragment shader reads. Must match
/// `SurfaceColor` in shaders.metal.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SurfaceColor {
    /// Columns applied to (y, cb, cr, 1) as sampled, range offsets included.
    pub ycbcr_to_rgb: [[f32; 4]; 4],
    /// Rows of the linear source-to-output primaries matrix.
    pub to_output_primaries: [[f32; 4]; 3],
    pub transfer: u32,
    pub is_rgb: u32,
    pub headroom: f32,
    pub content_peak: f32,
}

const TRANSFER_LEGACY: u32 = 0;
const TRANSFER_SDR: u32 = 1;
const TRANSFER_PQ: u32 = 2;
const TRANSFER_HLG: u32 = 3;

/// Reference white for HDR content (ITU-R BT.2408).
const SDR_WHITE_NITS: f32 = 203.0;
/// Assumed peak of HDR content that carries no light level metadata.
const DEFAULT_HDR_PEAK_NITS: f32 = 1000.0;

/// How a surface's planes are laid out for Metal.
pub(crate) enum SurfacePlanes {
    /// One packed RGB plane.
    Packed(MTLPixelFormat),
    /// A luma plane and an interleaved chroma plane.
    BiPlanar(MTLPixelFormat, MTLPixelFormat),
}

pub(crate) struct SurfaceFormat {
    pub planes: SurfacePlanes,
    pub color: SurfaceColor,
}

#[derive(Clone, Copy, PartialEq)]
enum Primaries {
    Bt709,
    P3,
    Bt2020,
}

impl SurfaceFormat {
    /// Reads the format and colour of `buffer`. `output_p3` is whether the
    /// target is tagged Display P3 (otherwise sRGB); `headroom` is the
    /// brightest value it can show, in units of SDR white.
    // CoreVideo's format constants are lowercase, which the lint takes for
    // bindings in match patterns.
    #[allow(non_upper_case_globals)]
    pub fn of(buffer: &CVPixelBuffer, output_p3: bool, headroom: f32) -> Result<Self> {
        let pixel_format = CVPixelBufferGetPixelFormatType(buffer);
        let (planes, bits, full_range) = match pixel_format {
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange => {
                return Ok(Self::legacy());
            }
            kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange => (
                SurfacePlanes::BiPlanar(MTLPixelFormat::R8Unorm, MTLPixelFormat::RG8Unorm),
                8,
                false,
            ),
            kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange => (
                SurfacePlanes::BiPlanar(MTLPixelFormat::R16Unorm, MTLPixelFormat::RG16Unorm),
                10,
                false,
            ),
            kCVPixelFormatType_420YpCbCr10BiPlanarFullRange => (
                SurfacePlanes::BiPlanar(MTLPixelFormat::R16Unorm, MTLPixelFormat::RG16Unorm),
                10,
                true,
            ),
            kCVPixelFormatType_32BGRA => (SurfacePlanes::Packed(MTLPixelFormat::BGRA8Unorm), 8, true),
            other => bail!("unsupported surface pixel format: {other:#x}"),
        };

        let transfer = transfer_of(buffer);
        let source_primaries = primaries_of(buffer);
        let output_primaries = if output_p3 { Primaries::P3 } else { Primaries::Bt709 };
        let is_rgb = matches!(planes, SurfacePlanes::Packed(_));

        let is_hdr = matches!(transfer, TRANSFER_PQ | TRANSFER_HLG);
        let (kr, kb) = if is_rgb { (0.2126, 0.0722) } else { matrix_of(buffer) };
        let color = SurfaceColor {
            ycbcr_to_rgb: ycbcr_to_rgb(kr, kb, bits, full_range),
            to_output_primaries: primaries_matrix(source_primaries, output_primaries),
            transfer,
            is_rgb: is_rgb as u32,
            headroom: headroom.max(1.0),
            content_peak: if is_hdr {
                DEFAULT_HDR_PEAK_NITS / SDR_WHITE_NITS
            } else {
                1.0
            },
        };
        Ok(Self { planes, color })
    }

    /// GPUI's original surface: full-range 8-bit NV12 with a fixed BT.601
    /// matrix and no colour management.
    fn legacy() -> Self {
        Self {
            planes: SurfacePlanes::BiPlanar(MTLPixelFormat::R8Unorm, MTLPixelFormat::RG8Unorm),
            color: SurfaceColor {
                ycbcr_to_rgb: [
                    [1.0, 1.0, 1.0, 0.0],
                    [0.0, -0.3441, 1.7720, 0.0],
                    [1.4020, -0.7141, 0.0, 0.0],
                    [-0.7010, 0.5291, -0.8860, 1.0],
                ],
                to_output_primaries: IDENTITY,
                transfer: TRANSFER_LEGACY,
                is_rgb: 0,
                headroom: 1.0,
                content_peak: 1.0,
            },
        }
    }
}

/// Whether `buffer` will want EDR output, without building its full colour.
pub(crate) fn is_hdr(buffer: &CVPixelBuffer) -> bool {
    CVPixelBufferGetPixelFormatType(buffer) != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        && matches!(transfer_of(buffer), TRANSFER_PQ | TRANSFER_HLG)
}

fn attachment(buffer: &CVPixelBuffer, key: &CFString) -> Option<CFRetained<CFType>> {
    // SAFETY: `key` is a live CFString and a null mode pointer is allowed.
    unsafe { buffer.attachment(key, ptr::null_mut()) }
}

fn attachment_is(value: &Option<CFRetained<CFType>>, expected: &CFString) -> bool {
    value
        .as_deref()
        .and_then(|value| value.downcast_ref::<CFString>())
        .is_some_and(|value| value == expected)
}

fn transfer_of(buffer: &CVPixelBuffer) -> u32 {
    // SAFETY: These are immutable CoreVideo constants.
    let (key, pq, hlg) = unsafe {
        (
            kCVImageBufferTransferFunctionKey,
            kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ,
            kCVImageBufferTransferFunction_ITU_R_2100_HLG,
        )
    };
    let value = attachment(buffer, key);
    if attachment_is(&value, pq) {
        TRANSFER_PQ
    } else if attachment_is(&value, hlg) {
        TRANSFER_HLG
    } else {
        TRANSFER_SDR
    }
}

fn primaries_of(buffer: &CVPixelBuffer) -> Primaries {
    // SAFETY: These are immutable CoreVideo constants.
    let (key, bt2020, p3, dci) = unsafe {
        (
            kCVImageBufferColorPrimariesKey,
            kCVImageBufferColorPrimaries_ITU_R_2020,
            kCVImageBufferColorPrimaries_P3_D65,
            kCVImageBufferColorPrimaries_DCI_P3,
        )
    };
    let value = attachment(buffer, key);
    if attachment_is(&value, bt2020) {
        Primaries::Bt2020
    } else if attachment_is(&value, p3) || attachment_is(&value, dci) {
        Primaries::P3
    } else {
        // BT.709, and the SD primaries close enough to it, or no tag.
        Primaries::Bt709
    }
}

/// Kr and Kb of the buffer's YCbCr matrix; BT.709 when untagged.
fn matrix_of(buffer: &CVPixelBuffer) -> (f32, f32) {
    // SAFETY: These are immutable CoreVideo constants.
    let (key, bt601, bt2020, smpte240) = unsafe {
        (
            kCVImageBufferYCbCrMatrixKey,
            kCVImageBufferYCbCrMatrix_ITU_R_601_4,
            kCVImageBufferYCbCrMatrix_ITU_R_2020,
            kCVImageBufferYCbCrMatrix_SMPTE_240M_1995,
        )
    };
    let value = attachment(buffer, key);
    if attachment_is(&value, bt601) {
        (0.299, 0.114)
    } else if attachment_is(&value, bt2020) {
        (0.2627, 0.0593)
    } else if attachment_is(&value, smpte240) {
        (0.212, 0.087)
    } else {
        (0.2126, 0.0722)
    }
}

/// The matrix the shader applies to (y, cb, cr, 1) as sampled. For 10-bit
/// formats the sample is the 16-bit container's value, with the code in its
/// top bits.
fn ycbcr_to_rgb(kr: f32, kb: f32, bits: u32, full_range: bool) -> [[f32; 4]; 4] {
    let scale = if bits == 10 { 65535.0 / 64.0 } else { 255.0 };
    let unit = (1u32 << (bits - 8)) as f32;
    let (y_offset, y_range, c_offset, c_range) = if full_range {
        let max = ((1u32 << bits) - 1) as f32;
        (0.0, max, 128.0 * unit, max)
    } else {
        (16.0 * unit, 219.0 * unit, 128.0 * unit, 224.0 * unit)
    };
    // y' = a_y * sample + b_y, and the same for both chroma channels.
    let (a_y, b_y) = (scale / y_range, -y_offset / y_range);
    let (a_c, b_c) = (scale / c_range, -c_offset / c_range);

    let kg = 1.0 - kr - kb;
    let cb = [0.0, -2.0 * kb * (1.0 - kb) / kg, 2.0 * (1.0 - kb)];
    let cr = [2.0 * (1.0 - kr), -2.0 * kr * (1.0 - kr) / kg, 0.0];
    let column = |coefficients: [f32; 3], a: f32| {
        [coefficients[0] * a, coefficients[1] * a, coefficients[2] * a, 0.0]
    };
    let offset = [
        b_y + b_c * (cb[0] + cr[0]),
        b_y + b_c * (cb[1] + cr[1]),
        b_y + b_c * (cb[2] + cr[2]),
        1.0,
    ];
    [column([1.0, 1.0, 1.0], a_y), column(cb, a_c), column(cr, a_c), offset]
}

const IDENTITY: [[f32; 4]; 3] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// Linear RGB to linear RGB between two sets of primaries (all D65).
fn primaries_matrix(from: Primaries, to: Primaries) -> [[f32; 4]; 3] {
    use Primaries::*;
    match (from, to) {
        (a, b) if a == b => IDENTITY,
        (Bt709, P3) => [
            [0.8225, 0.1774, 0.0000, 0.0],
            [0.0332, 0.9669, 0.0000, 0.0],
            [0.0171, 0.0724, 0.9108, 0.0],
        ],
        (Bt2020, P3) => [
            [1.3434, -0.2822, -0.0612, 0.0],
            [-0.0653, 1.0758, -0.0105, 0.0],
            [0.0028, -0.0196, 1.0168, 0.0],
        ],
        (P3, Bt709) => [
            [1.2249, -0.2247, 0.0000, 0.0],
            [-0.0420, 1.0419, 0.0000, 0.0],
            [-0.0197, -0.0786, 1.0979, 0.0],
        ],
        (Bt2020, Bt709) => [
            [1.6605, -0.5876, -0.0728, 0.0],
            [-0.1246, 1.1329, -0.0083, 0.0],
            [-0.0182, -0.1006, 1.1187, 0.0],
        ],
        // Only P3 and BT.709 are outputs.
        _ => IDENTITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: [[f32; 4]; 4], y: f32, cb: f32, cr: f32) -> [f32; 3] {
        let v = [y, cb, cr, 1.0];
        let mut out = [0.0; 3];
        for (row, value) in out.iter_mut().enumerate() {
            *value = (0..4).map(|column| m[column][row] * v[column]).sum();
        }
        out
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 2e-3)
    }

    #[test]
    fn video_range_8_bit_black_and_white() {
        let m = ycbcr_to_rgb(0.2126, 0.0722, 8, false);
        assert!(close(apply(m, 16.0 / 255.0, 0.5 * 256.0 / 255.0, 128.0 / 255.0), [0.0; 3]));
        assert!(close(apply(m, 235.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0), [1.0; 3]));
    }

    #[test]
    fn video_range_10_bit_white_and_red() {
        let m = ycbcr_to_rgb(0.2627, 0.0593, 10, false);
        let sample = |code: f32| code * 64.0 / 65535.0;
        assert!(close(apply(m, sample(940.0), sample(512.0), sample(512.0)), [1.0; 3]));
        // BT.2020 red: Y' = Kr, Cb' = -Kr / (2 (1 - Kb)), Cr' = 0.5.
        let (kr, kb) = (0.2627f32, 0.0593f32);
        let y = 64.0 + 876.0 * kr;
        let cb = 512.0 + 896.0 * (-kr / (2.0 * (1.0 - kb)));
        let cr = 512.0 + 896.0 * 0.5;
        assert!(close(apply(m, sample(y), sample(cb), sample(cr)), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn full_range_10_bit_white() {
        let m = ycbcr_to_rgb(0.2126, 0.0722, 10, true);
        let sample = |code: f32| code * 64.0 / 65535.0;
        assert!(close(apply(m, sample(1023.0), sample(512.0), sample(512.0)), [1.0; 3]));
    }
}
