//! Resample WASAPI's interleaved mix format before the fixed-rate Opus producer.

use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{
    Async, FixedAsync, Resampler as _, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

pub(super) struct RateConverter {
    resampler: Async<f32>,
    chunk_frames: usize,
    channels: usize,
    skip: usize,
    delay: usize,
    input_planar: Vec<Vec<f32>>,
    output_planar: Vec<Vec<f32>>,
    output_frames_max: usize,
    pending: Vec<f32>,
}

impl RateConverter {
    pub(super) fn new(
        input_rate: u32,
        output_rate: u32,
        channels: u32,
        chunk_frames: usize,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            chunk_frames > 0 && channels > 0,
            "invalid resampler dimensions"
        );
        let parameters = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: Some(0.95),
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        let resampler = Async::<f32>::new_sinc(
            f64::from(output_rate) / f64::from(input_rate),
            1.0,
            &parameters,
            chunk_frames,
            channels as usize,
            FixedAsync::Input,
        )?;
        let delay = resampler.output_delay();
        let output_frames_max = resampler.output_frames_max();
        Ok(Self {
            resampler,
            chunk_frames,
            channels: channels as usize,
            skip: delay,
            delay,
            input_planar: vec![vec![0.0; chunk_frames]; channels as usize],
            output_planar: vec![vec![0.0; output_frames_max]; channels as usize],
            output_frames_max,
            pending: Vec::new(),
        })
    }

    pub(super) fn reset(&mut self) {
        self.resampler.reset();
        self.pending.clear();
        self.skip = self.delay;
    }

    pub(super) fn process(&mut self, samples: &[f32]) -> anyhow::Result<Vec<f32>> {
        anyhow::ensure!(
            samples.len().is_multiple_of(self.channels),
            "misaligned resampler input"
        );
        self.pending.extend_from_slice(samples);
        let chunk_samples = self.chunk_frames * self.channels;
        let mut output = Vec::new();

        while self.pending.len() >= chunk_samples {
            for (frame_index, frame) in self.pending[..chunk_samples]
                .chunks_exact(self.channels)
                .enumerate()
            {
                for (channel, &sample) in frame.iter().enumerate() {
                    self.input_planar[channel][frame_index] = sample;
                }
            }
            let input =
                SequentialSliceOfVecs::new(&self.input_planar, self.channels, self.chunk_frames)?;
            let mut converted = SequentialSliceOfVecs::new_mut(
                &mut self.output_planar,
                self.channels,
                self.output_frames_max,
            )?;
            let (_, produced) = self
                .resampler
                .process_into_buffer(&input, &mut converted, None)?;
            let previous = output.len();
            output.resize(previous + produced * self.channels, 0.0);
            for frame_index in 0..produced {
                for channel in 0..self.channels {
                    output[previous + frame_index * self.channels + channel] =
                        self.output_planar[channel][frame_index];
                }
            }
            self.pending.drain(..chunk_samples);
        }

        // The sinc filter begins with its own centering delay, not captured audio.
        let skipped = self.skip.min(output.len() / self.channels) * self.channels;
        output.drain(..skipped);
        self.skip -= skipped / self.channels;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::RateConverter;

    #[test]
    fn converts_44100_stereo_and_resets_without_old_samples() {
        let mut converter = RateConverter::new(44_100, 48_000, 2, 441).unwrap();
        let input = vec![0.25; 441 * 2];
        let mut emitted = 0;
        for _ in 0..8 {
            let output = converter.process(&input).unwrap();
            emitted += output.len();
            assert!(output.len().is_multiple_of(2));
        }
        assert!(emitted > 0);
        converter.reset();
        assert!(converter.process(&[]).unwrap().is_empty());
        assert!(converter.process(&input[..100]).unwrap().is_empty());
    }
}
