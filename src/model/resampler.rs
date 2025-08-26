use rubato::{FftFixedIn, Resampler as RubatoResampler};
use symphonia::core::{
    audio::{AudioBuffer, Signal, SignalSpec},
    conv::IntoSample,
    sample::Sample,
};

use crate::model::device::BaseSample;

pub struct Resampler {
    resampler: FftFixedIn<BaseSample>,
    input: Vec<Vec<BaseSample>>,
    output: Vec<Vec<BaseSample>>,
    interleaved: Vec<BaseSample>,
    duration: usize,
}

impl Resampler {
    pub fn new(spec: SignalSpec, out_rate: u32, duration: u64, speed: u16) -> Self {
        let duration = duration as usize;
        let n_channels = spec.channels.count();
        let (in_rate, out_rate) = (
            (spec.rate as f32 * (speed as f32) / 100.0) as usize,
            out_rate as usize,
        );
        let resampler =
            FftFixedIn::<BaseSample>::new(in_rate, out_rate, duration, 2, n_channels).unwrap();
        let input = vec![Vec::with_capacity(duration); n_channels];
        let output = FftFixedIn::output_buffer_allocate(&resampler, true);
        let interleaved = Vec::new();

        Self {
            resampler,
            input,
            output,
            interleaved,
            duration,
        }
    }

    pub fn resample(&mut self, samples: &AudioBuffer<BaseSample>) -> Option<&[BaseSample]> {
        for (i, in_chan) in self.input.iter_mut().enumerate() {
            in_chan.extend(samples.chan(i).iter());
        }
        // not enough samples to succesfully resample
        if self.input[0].len() < self.duration {
            return None;
        }

        let (_, n_written) = {
            let mut in_channels = Vec::with_capacity(self.input.len());
            for channel in self.input.iter() {
                in_channels.push(&channel[..self.duration]);
            }

            self.resampler
                .process_into_buffer(&in_channels, &mut self.output, None)
                .unwrap()
        };
        for channel in self.input.iter_mut() {
            channel.drain(0..self.duration);
        }
        let num_channels = self.output.len();
        self.interleaved
            .resize(num_channels * n_written, BaseSample::MID);
        for (i, frame) in self.interleaved.chunks_exact_mut(num_channels).enumerate() {
            for (chan, s) in frame.iter_mut().enumerate() {
                *s = self.output[chan][i].into_sample();
            }
        }

        Some(&self.interleaved)
    }
}
