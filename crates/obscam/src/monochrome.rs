use zwo_asi::{FrameGeneration, HEIGHT, WIDTH};

const Y_BYTES: usize = WIDTH * HEIGHT;
const I420_BYTES: usize = Y_BYTES + Y_BYTES / 2;

/// Reconstructs neutral full-resolution monochrome into one reusable I420 buffer.
pub struct MonochromeProcessor {
    output: Box<[u8]>,
}

impl MonochromeProcessor {
    /// Allocates the single production processing buffer.
    #[must_use]
    pub fn new() -> Self {
        let mut output = vec![0; I420_BYTES].into_boxed_slice();
        output[Y_BYTES..].fill(128);
        Self { output }
    }

    /// Converts one validated RAW8 RGGB generation without an intermediate RGB frame.
    pub fn process<'a>(&'a mut self, source: &FrameGeneration<'_>) -> MonochromeFrame<'a> {
        self.process_validated(source.generation(), source.data())
    }

    pub(crate) fn process_validated<'a>(
        &'a mut self,
        generation: u64,
        raw: &[u8],
    ) -> MonochromeFrame<'a> {
        process_edges(raw, &mut self.output);
        process_interior(raw, &mut self.output);
        ProcessedFrame::new(generation, &self.output)
    }
}

fn process_edges(raw: &[u8], output: &mut [u8]) {
    for x in 0..WIDTH {
        output[x] = reconstructed_luminance(raw, x, 0);
        output[(HEIGHT - 1) * WIDTH + x] = reconstructed_luminance(raw, x, HEIGHT - 1);
    }
    for y in 1..HEIGHT - 1 {
        output[y * WIDTH] = reconstructed_luminance(raw, 0, y);
        output[y * WIDTH + WIDTH - 1] = reconstructed_luminance(raw, WIDTH - 1, y);
    }
}

fn process_interior(raw: &[u8], output: &mut [u8]) {
    for blue_y in (1..HEIGHT - 1).step_by(2) {
        let red_y = blue_y + 1;
        let above = row(raw, blue_y - 1);
        let blue_row = row(raw, blue_y);
        let red_row = row(raw, red_y);
        let below = row(raw, red_y + 1);

        for blue_x in (1..WIDTH - 1).step_by(2) {
            let red_x = blue_x + 1;

            output[blue_y * WIDTH + blue_x] = luminance(
                average4(
                    above[blue_x - 1],
                    above[blue_x + 1],
                    red_row[blue_x - 1],
                    red_row[blue_x + 1],
                ),
                average4(
                    blue_row[blue_x - 1],
                    blue_row[blue_x + 1],
                    above[blue_x],
                    red_row[blue_x],
                ),
                blue_row[blue_x],
            );
            output[blue_y * WIDTH + red_x] = luminance(
                average2(above[red_x], red_row[red_x]),
                blue_row[red_x],
                average2(blue_row[blue_x], blue_row[red_x + 1]),
            );
            output[red_y * WIDTH + blue_x] = luminance(
                average2(red_row[blue_x - 1], red_row[blue_x + 1]),
                red_row[blue_x],
                average2(blue_row[blue_x], below[blue_x]),
            );
            output[red_y * WIDTH + red_x] = luminance(
                red_row[red_x],
                average4(
                    red_row[blue_x],
                    red_row[red_x + 1],
                    blue_row[red_x],
                    below[red_x],
                ),
                average4(
                    blue_row[blue_x],
                    blue_row[red_x + 1],
                    below[blue_x],
                    below[red_x + 1],
                ),
            );
        }
    }
}

fn row(raw: &[u8], y: usize) -> &[u8] {
    &raw[y * WIDTH..(y + 1) * WIDTH]
}

fn reconstructed_luminance(raw: &[u8], x: usize, y: usize) -> u8 {
    let (red, green, blue) = reconstruct(raw, x, y);
    luminance(red, green, blue)
}

fn average2(first: u8, second: u8) -> u8 {
    u8::try_from(u16::midpoint(u16::from(first), u16::from(second)))
        .expect("the midpoint of RAW8 samples remains RAW8")
}

fn average4(first: u8, second: u8, third: u8, fourth: u8) -> u8 {
    let average = (u16::from(first) + u16::from(second) + u16::from(third) + u16::from(fourth)) / 4;
    u8::try_from(average).expect("the average of RAW8 samples remains RAW8")
}

impl Default for MonochromeProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// One native-dimension neutral monochrome I420 generation.
pub struct ProcessedFrame<'a> {
    generation: u64,
    data: &'a [u8],
}

impl<'a> ProcessedFrame<'a> {
    pub(crate) const fn new(generation: u64, data: &'a [u8]) -> Self {
        Self { generation, data }
    }

    /// Source generation represented by this complete output frame.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Native frame width.
    #[must_use]
    pub const fn width(&self) -> usize {
        WIDTH
    }

    /// Native frame height.
    #[must_use]
    pub const fn height(&self) -> usize {
        HEIGHT
    }

    /// Contiguous full-resolution I420 bytes: Y, then neutral U and V planes.
    #[must_use]
    pub const fn data(&self) -> &[u8] {
        self.data
    }
}

pub(crate) fn reconstruct(raw: &[u8], x: usize, y: usize) -> (u8, u8, u8) {
    match (x & 1, y & 1) {
        (0, 0) => (
            sample(raw, x, y),
            axial_average(raw, x, y),
            diagonal_average(raw, x, y),
        ),
        (1, 0) => (
            horizontal_average(raw, x, y),
            sample(raw, x, y),
            vertical_average(raw, x, y),
        ),
        (0, 1) => (
            vertical_average(raw, x, y),
            sample(raw, x, y),
            horizontal_average(raw, x, y),
        ),
        (1, 1) => (
            diagonal_average(raw, x, y),
            axial_average(raw, x, y),
            sample(raw, x, y),
        ),
        _ => unreachable!(),
    }
}

fn horizontal_average(raw: &[u8], x: usize, y: usize) -> u8 {
    neighbor_average(raw, x, y, &[(-1, 0), (1, 0)])
}

fn vertical_average(raw: &[u8], x: usize, y: usize) -> u8 {
    neighbor_average(raw, x, y, &[(0, -1), (0, 1)])
}

fn axial_average(raw: &[u8], x: usize, y: usize) -> u8 {
    neighbor_average(raw, x, y, &[(-1, 0), (1, 0), (0, -1), (0, 1)])
}

fn diagonal_average(raw: &[u8], x: usize, y: usize) -> u8 {
    neighbor_average(raw, x, y, &[(-1, -1), (1, -1), (-1, 1), (1, 1)])
}

fn neighbor_average(raw: &[u8], x: usize, y: usize, offsets: &[(isize, isize)]) -> u8 {
    let mut sum = 0_u16;
    let mut count = 0_u16;
    for &(x_offset, y_offset) in offsets {
        if let (Some(sample_x), Some(sample_y)) = (
            x.checked_add_signed(x_offset)
                .filter(|value| *value < WIDTH),
            y.checked_add_signed(y_offset)
                .filter(|value| *value < HEIGHT),
        ) {
            sum += u16::from(sample(raw, sample_x, sample_y));
            count += 1;
        }
    }
    u8::try_from(sum / count).expect("an average of RAW8 samples remains RAW8")
}

fn sample(raw: &[u8], x: usize, y: usize) -> u8 {
    raw[y * WIDTH + x]
}

pub(crate) const fn luminance(red: u8, green: u8, blue: u8) -> u8 {
    let weighted = 77 * red as u16 + 150 * green as u16 + 29 * blue as u16;
    ((weighted + 128) >> 8) as u8
}

/// Backward-compatible name for the neutral monochrome processing result.
pub type MonochromeFrame<'a> = ProcessedFrame<'a>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_processing_matches_the_bilinear_reference_at_every_pixel() {
        let raw = (0..HEIGHT)
            .flat_map(|y| {
                (0..WIDTH).map(move |x| {
                    u8::try_from((x * 17 + y * 31 + (x * y) % 251) % 256)
                        .expect("pattern value is bounded to RAW8")
                })
            })
            .collect::<Vec<_>>();
        let mut processor = MonochromeProcessor::new();
        let output = processor.process_validated(1, &raw);

        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                assert_eq!(
                    output.data()[y * WIDTH + x],
                    reconstructed_luminance(&raw, x, y),
                    "bilinear luminance mismatch at ({x}, {y})"
                );
            }
        }
    }
}
