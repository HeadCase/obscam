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
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let (red, green, blue) = reconstruct(raw, x, y);
                self.output[y * WIDTH + x] = luminance(red, green, blue);
            }
        }
        MonochromeFrame {
            generation,
            data: &self.output,
        }
    }
}

impl Default for MonochromeProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// One native-dimension neutral monochrome I420 generation.
pub struct MonochromeFrame<'a> {
    generation: u64,
    data: &'a [u8],
}

impl MonochromeFrame<'_> {
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

fn reconstruct(raw: &[u8], x: usize, y: usize) -> (u8, u8, u8) {
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

const fn luminance(red: u8, green: u8, blue: u8) -> u8 {
    let weighted = 77 * red as u16 + 150 * green as u16 + 29 * blue as u16;
    ((weighted + 128) >> 8) as u8
}
