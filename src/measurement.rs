use std::collections::VecDeque;

use crate::types::Metadata;

#[derive(Clone, Debug)]
pub struct MeasurementMissed {
    pub expected_counter: Option<u8>,
    pub actual_counter: u8,
}

pub type Result = std::result::Result<Measurement, MeasurementMissed>;

const ADC_MULTIPLIER: f32 = 1.8 / 163840.;
const SPIKE_FILTER_ALPHA: f32 = 0.18;
const SPIKE_FILTER_ALPHA_5: f32 = 0.06;
const SPIKE_FILTER_SAMPLES: isize = 3;

#[derive(Debug)]
pub struct Measurement {
    pub counter: u8,
    pub analog_value: f32,
    pub bits: u32,
}

struct AccumulatorState {
    rolling_avg_4: Option<f32>,
    rolling_avg: Option<f32>,
    prev_range: Option<usize>,
    after_spike: isize,
    consecutive_range_sample: usize,
    expected_counter: Option<u8>,
}

pub struct MeasurementAccumulator {
    state: AccumulatorState,
    buf: Vec<u8>,
    metadata: Metadata,
}

impl MeasurementAccumulator {
    pub fn new(metadata: Metadata) -> Self {
        Self {
            metadata,
            state: AccumulatorState {
                rolling_avg_4: None,
                rolling_avg: None,
                prev_range: None,
                after_spike: 0,
                consecutive_range_sample: 0,
                expected_counter: None,
            },
            buf: Vec::with_capacity(1024),
        }
    }

    pub fn feed_into(&mut self, bytes: &[u8], buf: &mut VecDeque<Result>) {
        if bytes.is_empty() {
            return;
        }
        self.buf.extend_from_slice(bytes);
        let chunks = self.buf.chunks_exact(4).map(|c| c.try_into().unwrap());
        for chunk in chunks {
            let raw = u32::from_le_bytes(chunk); // Not sure if LE or BE
            let current_measurement_range = get_range(raw) as usize;
            let counter = get_counter(raw) as u8;
            let adc_result = get_adc(raw) * 4;
            let bits = get_logic(raw);
            let analog_value = get_adc_result(
                &self.metadata,
                &mut self.state,
                current_measurement_range,
                adc_result,
            ) * 10f32.powi(6);
            if self.state.expected_counter.is_none() {
                self.state.expected_counter.replace(counter);
            }
            let prev_expected_counter = self.state.expected_counter;
            self.state.expected_counter.replace((counter + 1) % 64);
            if prev_expected_counter != Some(counter) {
                buf.push_back(Err(MeasurementMissed {
                    expected_counter: prev_expected_counter,
                    actual_counter: counter,
                }))
            }
            buf.push_back(Ok(Measurement {
                counter,
                analog_value,
                bits,
            }))
        }
    }
}

fn get_adc_result(
    metadata: &Metadata,
    state: &mut AccumulatorState,
    range: usize,
    adc_val: u32,
) -> f32 {
    let modifiers = &metadata.modifiers;

    let result_without_gain: f32 =
        (adc_val as f32 - modifiers.o[range]) * (ADC_MULTIPLIER / modifiers.r[range]);
    let mut adc = modifiers.ug[range]
        * (result_without_gain * (modifiers.gs[range] * result_without_gain + modifiers.gi[range])
            + (modifiers.s[range] * (metadata.vdd as f32 / 1000.) + modifiers.i[range]));

    let prev_rolling_avg_4 = state.rolling_avg_4;
    let prev_rolling_avg = state.rolling_avg;

    state
        .rolling_avg
        .replace(if let Some(rolling_avg) = state.rolling_avg {
            SPIKE_FILTER_ALPHA * adc + (1. - SPIKE_FILTER_ALPHA) * rolling_avg
        } else {
            adc
        });

    state
        .rolling_avg_4
        .replace(if let Some(rolling_avg_4) = state.rolling_avg_4 {
            SPIKE_FILTER_ALPHA_5 * adc + (1. - SPIKE_FILTER_ALPHA_5) * rolling_avg_4
        } else {
            adc
        });

    if state.prev_range.is_none() {
        state.prev_range.replace(range);
    }

    if !matches!(state.prev_range, Some(r) if r == range) || state.after_spike > 0 {
        if matches!(state.prev_range, Some(r) if r == range) {
            state.consecutive_range_sample = 0;
            state.after_spike = SPIKE_FILTER_SAMPLES;
        } else {
            state.consecutive_range_sample += 1;
        }

        if range == 4 {
            if state.consecutive_range_sample < 2 {
                state.rolling_avg_4 = prev_rolling_avg_4;
                state.rolling_avg = prev_rolling_avg;
            }
            adc = state.rolling_avg_4.unwrap();
        } else {
            adc = state.rolling_avg.unwrap();
        }
        state.after_spike -= 1;
    }
    state.prev_range = Some(range);
    adc
}

const fn generate_mask(bits: u32, pos: u32) -> u32 {
    (2u32.pow(bits as u32) - 1) << pos
}

macro_rules! masked_value {
    ($name:ident, $bits:literal, $pos:literal) => {
        fn $name(raw: u32) -> u32 {
            (raw & generate_mask($bits, $pos)) >> $pos
        }
    };
}

masked_value!(get_adc, 14, 0);
masked_value!(get_range, 3, 14);
masked_value!(get_counter, 6, 18);
masked_value!(get_logic, 8, 24);
