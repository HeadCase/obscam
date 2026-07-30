use zwo_asi::{FrameGeneration, HEIGHT, WIDTH};

use crate::monochrome::{ProcessedFrame, luminance, reconstruct};

const Y_BYTES: usize = WIDTH * HEIGHT;
const CHROMA_WIDTH: usize = WIDTH / 2;
const CHROMA_BYTES: usize = Y_BYTES / 4;
const I420_BYTES: usize = Y_BYTES + CHROMA_BYTES * 2;

/// Converts RAW8 RGGB directly into one reusable full-resolution colour I420 buffer.
pub struct ColourProcessor {
    output: Box<[u8]>,
}

impl ColourProcessor {
    /// Allocates the single production colour-processing buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            output: vec![0; I420_BYTES].into_boxed_slice(),
        }
    }

    /// Applies fixed bilinear reconstruction and full-range BT.601 conversion.
    pub fn process<'a>(&'a mut self, source: &FrameGeneration<'_>) -> ProcessedFrame<'a> {
        self.process_validated(source.generation(), source.data())
    }

    pub(crate) fn process_validated<'a>(
        &'a mut self,
        generation: u64,
        raw: &[u8],
    ) -> ProcessedFrame<'a> {
        for y in (0..HEIGHT).step_by(2) {
            for x in (0..WIDTH).step_by(2) {
                let mut red = 0_u16;
                let mut green = 0_u16;
                let mut blue = 0_u16;
                for offset_y in 0..2 {
                    for offset_x in 0..2 {
                        let sample_x = x + offset_x;
                        let sample_y = y + offset_y;
                        let (r, g, b) = reconstruct(raw, sample_x, sample_y);
                        self.output[sample_y * WIDTH + sample_x] = luminance(r, g, b);
                        red += u16::from(r);
                        green += u16::from(g);
                        blue += u16::from(b);
                    }
                }
                let red = i32::from(red / 4);
                let green = i32::from(green / 4);
                let blue = i32::from(blue / 4);
                let chroma_index = (y / 2) * CHROMA_WIDTH + x / 2;
                self.output[Y_BYTES + chroma_index] = chroma(-43 * red - 85 * green + 128 * blue);
                self.output[Y_BYTES + CHROMA_BYTES + chroma_index] =
                    chroma(128 * red - 107 * green - 21 * blue);
            }
        }
        ProcessedFrame::new(generation, &self.output)
    }
}

impl Default for ColourProcessor {
    fn default() -> Self {
        Self::new()
    }
}

fn chroma(weighted: i32) -> u8 {
    let rounded = if weighted >= 0 {
        (weighted + 128) / 256
    } else {
        (weighted - 128) / 256
    };
    u8::try_from((rounded + 128).clamp(0, 255)).expect("clamped BT.601 chroma fits u8")
}
