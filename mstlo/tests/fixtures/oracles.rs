#![allow(dead_code)]

use mstlo::ring_buffer::Step;
use mstlo::step;
use rstest::fixture;
use std::time::Duration;

use crate::common::convert_f64_vec_to_bool_vec;

// ---
// Expected Result "Oracles" (Plain functions)
// ---
#[fixture]
#[once]
pub fn exp_f1_s1_f64_delayed() -> Vec<Vec<Step<f64>>> {
    vec![
        vec![],
        vec![],
        vec![step!("output", 1.0, Duration::from_secs(0))],
        vec![step!("output", -1.0, Duration::from_secs(1))],
        vec![step!("output", -1.0, Duration::from_secs(2))],
    ]
}

pub fn exp_f1_s1_bool_delayed() -> Vec<Vec<Step<bool>>> {
    convert_f64_vec_to_bool_vec(exp_f1_s1_f64_delayed())
}

pub fn exp_f1_s1_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![],
        vec![],
        vec![step!("output", true, Duration::from_secs(0))],
        vec![
            step!("output", false, Duration::from_secs(1)),
            step!("output", false, Duration::from_secs(2)),
            step!("output", false, Duration::from_secs(3)),
        ],
        vec![],
    ]
}

pub fn exp_f2_s2_f64_delayed() -> Vec<Vec<Step<f64>>> {
    vec![
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![step!("output", 1.0, Duration::from_secs(0))],
        vec![step!("output", 1.0, Duration::from_secs(1))],
        vec![step!("output", 1.0, Duration::from_secs(2))],
    ]
}

pub fn exp_f2_s2_bool_delayed() -> Vec<Vec<Step<bool>>> {
    convert_f64_vec_to_bool_vec(exp_f2_s2_f64_delayed())
}

pub fn exp_f2_s2_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![
            step!("output", true, Duration::from_secs(0)),
            step!("output", true, Duration::from_secs(1)),
            step!("output", true, Duration::from_secs(2)),
            step!("output", true, Duration::from_secs(3)),
        ],
        vec![
            step!("output", false, Duration::from_secs(4)),
            step!("output", false, Duration::from_secs(5)),
        ],
        vec![],
        vec![step!("output", false, Duration::from_secs(6))],
        vec![step!("output", false, Duration::from_secs(7))],
        vec![step!("output", false, Duration::from_secs(8))],
    ]
}

pub fn exp_f3_s3_f64_delayed() -> Vec<Vec<Step<f64>>> {
    vec![
        vec![],
        vec![],
        vec![step!("output", 0.0, Duration::from_secs(0))],
        vec![step!("output", 0.0, Duration::from_secs(1))],
        vec![step!("output", 0.0, Duration::from_secs(2))],
        vec![step!("output", 0.0, Duration::from_secs(3))],
        vec![step!("output", 1.0, Duration::from_secs(4))],
    ]
}

pub fn exp_f3_s3_bool_delayed() -> Vec<Vec<Step<bool>>> {
    convert_f64_vec_to_bool_vec(exp_f3_s3_f64_delayed())
}

pub fn exp_f3_s3_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![step!("output", false, Duration::from_secs(0))],
        vec![],
        vec![],
        vec![
            step!("output", false, Duration::from_secs(1)),
            step!("output", false, Duration::from_secs(2)),
            step!("output", false, Duration::from_secs(3)),
        ],
        vec![],
        vec![],
        vec![step!("output", true, Duration::from_secs(4))],
    ]
}

pub fn exp_f4_s3_f64_delayed() -> Vec<Vec<Step<f64>>> {
    vec![
        vec![],
        vec![],
        vec![step!("output", 1.0, Duration::from_secs(0))],
        vec![step!("output", 1.0, Duration::from_secs(1))],
        vec![step!("output", 3.0, Duration::from_secs(2))],
        vec![step!("output", 3.0, Duration::from_secs(3))],
        vec![step!("output", 3.0, Duration::from_secs(4))],
    ]
}

pub fn exp_f4_s3_bool_delayed() -> Vec<Vec<Step<bool>>> {
    convert_f64_vec_to_bool_vec(exp_f4_s3_f64_delayed())
}

pub fn exp_f4_s3_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![],
        vec![
            step!("output", true, Duration::from_secs(0)),
            step!("output", true, Duration::from_secs(1)),
        ],
        vec![],
        vec![],
        vec![
            step!("output", true, Duration::from_secs(2)),
            step!("output", true, Duration::from_secs(3)),
            step!("output", true, Duration::from_secs(4)),
        ],
        vec![],
        vec![
            step!("output", true, Duration::from_secs(5)),
            step!("output", true, Duration::from_secs(6)),
        ],
    ]
}

pub fn exp_f6_s2_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![
            // step t=5s
            step!("output", false, Duration::from_secs(0)),
        ],
        vec![
            // step t=6s, x=0.0, G[0,5](x>0) is false => short-circuit to true
            step!("output", true, Duration::from_secs(1)),
            step!("output", true, Duration::from_secs(2)),
            step!("output", true, Duration::from_secs(3)),
            step!("output", true, Duration::from_secs(4)),
            step!("output", true, Duration::from_secs(5)),
            step!("output", true, Duration::from_secs(6)),
        ],
        vec![step!("output", true, Duration::from_secs(7))],
        vec![step!("output", true, Duration::from_secs(8))],
        vec![],
        vec![],
    ]
}
pub fn exp_f6_s2_f64_delayed() -> Vec<Vec<Step<f64>>> {
    vec![
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![step!("output", -1.0, Duration::from_secs(0))],
        vec![step!("output", -0.0, Duration::from_secs(1))],
        vec![step!("output", 0.0, Duration::from_secs(2))],
        vec![step!("output", 1.0, Duration::from_secs(3))],
        vec![step!("output", 1.0, Duration::from_secs(4))],
        vec![step!("output", 1.0, Duration::from_secs(5))],
    ]
}
pub fn exp_f6_s2_bool_delayed() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![step!("output", false, Duration::from_secs(0))],
        vec![step!("output", true, Duration::from_secs(1))],
        vec![step!("output", true, Duration::from_secs(2))],
        vec![step!("output", true, Duration::from_secs(3))],
        vec![step!("output", true, Duration::from_secs(4))],
        vec![step!("output", true, Duration::from_secs(5))],
    ]
}

pub fn exp_f7_s3_f64_delayed() -> Vec<Vec<Step<f64>>> {
    vec![
        vec![step!("output", -5.0, Duration::from_secs(0))],
        vec![step!("output", 1.0, Duration::from_secs(1))],
        vec![step!("output", -4.0, Duration::from_secs(2))],
        vec![step!("output", -5.0, Duration::from_secs(3))],
        vec![step!("output", 3.0, Duration::from_secs(4))],
        vec![step!("output", -4.0, Duration::from_secs(5))],
        vec![step!("output", 2.0, Duration::from_secs(6))],
    ]
}

pub fn exp_f7_s3_bool_delayed() -> Vec<Vec<Step<bool>>> {
    convert_f64_vec_to_bool_vec(exp_f7_s3_f64_delayed())
}

pub fn exp_f7_s3_bool_eager() -> Vec<Vec<Step<bool>>> {
    exp_f7_s3_bool_delayed()
}

pub fn exp_f8_s4_f64_delayed() -> Vec<Vec<Step<f64>>> {
    vec![
        vec![],                                              // x@t=0
        vec![],                                              // y@t=0
        vec![],                                              // x@t=1
        vec![],                                              // y@t=1
        vec![step!("output", 0.0, Duration::from_secs(0))],  // x@t=2
        vec![],                                              // y@t=2
        vec![step!("output", 1.0, Duration::from_secs(1))],  // x@t=3
        vec![],                                              // y@t=3
        vec![step!("output", -1.0, Duration::from_secs(2))], // x@t=4
        vec![],                                              // y@t=5
        vec![step!("output", -2.0, Duration::from_secs(3))], // x@t=5
        vec![],                                              // y@t=4
        vec![step!("output", 1.0, Duration::from_secs(4))],  // x@t=6
        vec![],                                              // y@t=6
    ]
}

pub fn exp_f8_s4_bool_delayed() -> Vec<Vec<Step<bool>>> {
    convert_f64_vec_to_bool_vec(exp_f8_s4_f64_delayed())
}

pub fn exp_f8_s4_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![step!("output", false, Duration::from_secs(0))], // x@t=0
        vec![],                                               // y@t=0
        vec![],                                               // x@t=1
        vec![],                                               // y@t=1
        vec![],                                               // x@t=2
        vec![],                                               // y@t=2
        vec![
            step!("output", true, Duration::from_secs(1)), // x@t=3
            step!("output", false, Duration::from_secs(2)),
        ],
        vec![step!("output", false, Duration::from_secs(3))], // y@t=3
        vec![],                                               // x@t=4
        vec![],                                               // y@t=4
        vec![],                                               // x@t=5
        vec![],                                               // y@t=5
        vec![step!("output", true, Duration::from_secs(4))],  // x@t=6
        vec![],                                               // y@t=6
    ]
}

pub fn exp_f10_s3_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![step!("output", false, Duration::from_secs(0))],
        vec![step!("output", false, Duration::from_secs(1))],
        vec![step!("output", false, Duration::from_secs(2))],
        vec![step!("output", false, Duration::from_secs(3))],
        vec![step!("output", false, Duration::from_secs(4))],
        vec![step!("output", false, Duration::from_secs(5))],
        vec![step!("output", false, Duration::from_secs(6))],
    ]
}

pub fn exp_f11_s3_bool_eager() -> Vec<Vec<Step<bool>>> {
    vec![
        vec![],
        vec![
            step!("output", true, Duration::from_secs(0)),
            step!("output", true, Duration::from_secs(1)),
        ],
        vec![],
        vec![],
        vec![
            step!("output", false, Duration::from_secs(2)),
            step!("output", false, Duration::from_secs(3)),
            step!("output", false, Duration::from_secs(4)),
        ],
        vec![],
        vec![
            step!("output", true, Duration::from_secs(5)),
            step!("output", true, Duration::from_secs(6)),
        ],
    ]
}
