use flate2::{Decompress, FlushDecompress, Status};

pub(crate) struct Decoder {
    inner: Decompress,
    gzip: bool,
    pub complete: bool,
    pub limited: bool,
    pub limit: u64,
}
impl Decoder {
    pub fn new(encoding: &str, limit: u64) -> Option<Self> {
        let gzip = encoding.eq_ignore_ascii_case("gzip");
        if !gzip && !encoding.eq_ignore_ascii_case("deflate") {
            return None;
        }
        Some(Self {
            inner: if gzip {
                Decompress::new_gzip(15)
            } else {
                Decompress::new(true)
            },
            gzip,
            complete: false,
            limited: false,
            limit,
        })
    }
    pub fn feed(
        &mut self,
        mut input: &[u8],
        mut write: impl FnMut(&[u8]) -> Result<bool, String>,
    ) -> Result<(), String> {
        if self.limited {
            return Ok(());
        }
        let mut output = [0; 64 * 1024];
        loop {
            if self.complete {
                if input.is_empty() {
                    return Ok(());
                }
                self.inner = if self.gzip {
                    Decompress::new_gzip(15)
                } else {
                    Decompress::new(true)
                };
                self.complete = false;
            }
            let before_in = self.inner.total_in();
            let before_out = self.inner.total_out();
            let status = self
                .inner
                .decompress(input, &mut output, FlushDecompress::None)
                .map_err(|_| "decode-failed")?;
            let consumed = (self.inner.total_in() - before_in) as usize;
            let produced = (self.inner.total_out() - before_out) as usize;
            if produced != 0 && !write(&output[..produced])? {
                self.limited = true;
                return Ok(());
            }
            input = &input[consumed..];
            self.complete = status == Status::StreamEnd;
            if (consumed == 0 && produced == 0) || (input.is_empty() && produced < output.len()) {
                return Ok(());
            }
        }
    }
}
