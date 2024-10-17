use pbc_zk::*;

/// Output variable type
#[derive(pbc_zk::SecretBinary, Clone)]
pub struct RandomnessInput {
    /// Coin flip result.
    pub result: Sbi8,
}


/// Perform a zk computation on secret-shared randomness to make a random coin flip.
///
/// ### Returns:
///
/// The sum of the randomness contributions variables, reduced to 0 or 1.
#[zk_compute(shortname = 0x61)]
pub fn compute_coin_flip() -> RandomnessInput 
{
    let mut flip = RandomnessInput {
        result: Sbi8::from(0),
    };

    for variable_id in secret_variable_ids() {
        let raw_contribution: RandomnessInput = load_sbi::<RandomnessInput>(variable_id);
        let result_reduced = reduce_contribution(raw_contribution.result);
        flip.result = flip.result + result_reduced;
    }

   
    // Perform the modulo operation directly on the Sbi8 value
    flip.result = flip.result & Sbi8::from(1); // Reduce the sum to 0 or 1
    flip
}


/// Reduce the contribution to 0 or 1.
fn reduce_contribution(value: Sbi8) -> Sbi8 {
    let reduced = value & Sbi8::from(0b111);
    if reduced >= Sbi8::from(2) {
        reduced - Sbi8::from(2)
    } else {
        reduced
    }
}

// Parse the output of the zk computation.
pub fn parse_compute_output(output: RandomnessInput) -> Sbi1 {
    output.result != Sbi8::from(0)
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct CoinFlipResult {
    pub id: u64,
    pub result: bool,
}