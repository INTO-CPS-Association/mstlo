#![allow(dead_code)]

use mstlo::stl;
use mstlo::formula_definition::FormulaDefinition;
use rstest::fixture;

// ---
// Formula Fixtures
// ---

#[fixture]
#[once]
pub fn formula_1() -> FormulaDefinition {
    stl! {
        G[0,2] (x > 3)
    }
}

#[fixture]
#[once]
pub fn formula_1_alt() -> FormulaDefinition {
    // G[a,b](phi) is equivalent to !(F[a,b]( ! (phi)))
    stl! {
        !(F[0,2] (x <= 3))
    }
}

#[fixture]
#[once]
pub fn formula_1_alt_2() -> FormulaDefinition {
    // F[a,b](phi) is equivalent to (true) U[a,b] (phi), so G[a,b](phi) is also equivalent to !( (true) U[a,b] ( ! (phi)))
    stl! {
        !( (true) U[0,2] (x <= 3))
    }
}

#[fixture]
#[once]
pub fn formula_2() -> FormulaDefinition {
    stl! {(G[0,2] (x > 0)) U[0,6] (F[0,2] (x > 3))}
}

#[fixture]
#[once]
pub fn formula_3() -> FormulaDefinition {
    stl! {(F[0,2] (x > 5)) && (G[0, 2] (x > 0))}
}

#[fixture]
#[once]
pub fn formula_3_alt() -> FormulaDefinition {
    // G[a,b](phi) is equivalent to !(F[a,b]( ! (phi)))
    stl! {((true) U[0,2] (x > 5)) && (!(F[0,2] (x <= 0)))}
}

#[fixture]
#[once]
pub fn formula_4() -> FormulaDefinition {
    stl! {(F[0, 2] (x > 5)) && (true)}
}

#[fixture]
#[once]
pub fn formula_5() -> FormulaDefinition {
    stl! {F[0, 2] (x > 5)}
}

#[fixture]
#[once]
pub fn formula_5_alt() -> FormulaDefinition {
    // F[a,b](phi) is equivalent to (true) U[a,b] (phi)
    stl! {(true) U[0, 2] (x > 5)}
}

#[fixture]
#[once]
pub fn formula_6() -> FormulaDefinition {
    stl! {(G[0, 5] (x > 0)) -> (F[0, 2] (x > 3))}
}

#[fixture]
#[once]
pub fn formula_6_alt() -> FormulaDefinition {
    // using rules for globally (in terms of eventually) and eventually (in terms of until)
    stl! {
        (!(F[0,5](x <= 0))) -> ((true) U[0,2](x > 3))
    }
}

#[fixture]
#[once]
pub fn formula_7() -> FormulaDefinition {
    stl! {(!(x < 5)) || (false)}
}

#[fixture]
#[once]
pub fn formula_8() -> FormulaDefinition {
    stl! {(G[0,2](x>0)) && (y<5)}
}

#[fixture]
#[once]
pub fn formula_8_alt() -> FormulaDefinition {
    stl! {(!(F[0,2](x<=0))) && (y<5)}
}

#[fixture]
#[once]
pub fn formula_9() -> FormulaDefinition {
    stl! {F[0, 10](G[0, 10](F[0, 10](G[0, 10](x > 0))))}
}

#[fixture]
#[once]
pub fn formula_10() -> FormulaDefinition {
    stl! {x<-1 U[0,4](x>-1)}
}

#[fixture]
#[once]
pub fn formula_11() -> FormulaDefinition {
    stl! {x<8 U[0,4] (x>5)}
}
