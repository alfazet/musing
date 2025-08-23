use rubato::{FftFixedIn, Resampler as RubatoResampler};
use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, Signal, SignalSpec},
    conv::{FromSample, IntoSample},
    sample::Sample,
};

use crate::model::device::BaseSample;

pub struct Resampler<T> {
    resampler: FftFixedIn<BaseSample>,
    input: Vec<Vec<BaseSample>>,
    output: Vec<Vec<BaseSample>>,
    interleaved: Vec<T>,
    duration: usize,
}

impl<T> Resampler<T>
where
    T: Sample + FromSample<BaseSample> + IntoSample<BaseSample>,
{
    pub fn new(spec: SignalSpec, out_rate: u32, duration: u64) -> Self {
        let duration = duration as usize;
        let n_channels = spec.channels.count();
        let (in_rate, out_rate) = (spec.rate as usize, out_rate as usize);
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

    pub fn resample(&mut self, samples: &AudioBufferRef<'_>) -> Option<&[T]> {
        convert_samples(samples, &mut self.input);
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
        self.interleaved.resize(num_channels * n_written, T::MID);
        for (i, frame) in self.interleaved.chunks_exact_mut(num_channels).enumerate() {
            for (chan, s) in frame.iter_mut().enumerate() {
                *s = self.output[chan][i].into_sample();
            }
        }

        Some(&self.interleaved)
    }
}

fn convert_samples(input: &AudioBufferRef<'_>, output: &mut [Vec<BaseSample>]) {
    match input {
        AudioBufferRef::U8(input) => convert_samples_inner(input, output),
        AudioBufferRef::U16(input) => convert_samples_inner(input, output),
        AudioBufferRef::U24(input) => convert_samples_inner(input, output),
        AudioBufferRef::U32(input) => convert_samples_inner(input, output),
        AudioBufferRef::S8(input) => convert_samples_inner(input, output),
        AudioBufferRef::S16(input) => convert_samples_inner(input, output),
        AudioBufferRef::S24(input) => convert_samples_inner(input, output),
        AudioBufferRef::S32(input) => convert_samples_inner(input, output),
        AudioBufferRef::F32(input) => convert_samples_inner(input, output),
        AudioBufferRef::F64(input) => convert_samples_inner(input, output),
    }
}

// convert samples to the type expected by the resampler
fn convert_samples_inner<T>(input: &AudioBuffer<T>, output: &mut [Vec<BaseSample>])
where
    T: Sample + IntoSample<BaseSample>,
{
    for (i, out_chan) in output.iter_mut().enumerate() {
        let in_chan = input.chan(i);
        out_chan.extend(in_chan.iter().map(|&s| s.into_sample()));
    }
}
