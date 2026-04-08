use mstlo::ring_buffer::Step;
use mstlo::step;
use std::time::Duration;

pub fn convert_f64_vec_to_bool_vec(input: Vec<Vec<Step<f64>>>) -> Vec<Vec<Step<bool>>> {
    input
        .into_iter()
        .map(|inner_vec| {
            inner_vec
                .into_iter()
                .map(|step| {
                    let bool_value =
                        step.value > 0.0 || (step.value == 0.0 && step.value.is_sign_negative());
                    step!("output", bool_value, step.timestamp)
                })
                .collect()
        })
        .collect()
}

// Helper to create a vector of steps
pub fn create_steps(name: &'static str, values: Vec<f64>, timestamps: Vec<u64>) -> Vec<Step<f64>> {
    values
        .into_iter()
        .zip(timestamps)
        .map(|(val, ts)| step!(name, val, Duration::from_secs(ts)))
        .collect()
}

pub fn combine_and_sort_steps(step_vectors: Vec<Vec<Step<f64>>>) -> Vec<Step<f64>> {
    let mut combined_steps = step_vectors.into_iter().flatten().collect::<Vec<_>>();
    combined_steps.sort_by_key(|step| step.timestamp);
    combined_steps
}
