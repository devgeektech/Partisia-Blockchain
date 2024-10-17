#![doc = include_str!("../README.md")]
#![allow(unused_variables)]

#[macro_use]
extern crate pbc_contract_codegen;
extern crate pbc_contract_common;
extern crate pbc_lib;

mod zk_compute;

use create_type_spec_derive::CreateTypeSpec;
use pbc_contract_common::address::Address;
use pbc_contract_common::context::ContractContext;
use pbc_contract_common::events::EventGroup;
use pbc_contract_common::sorted_vec_map::SortedVecMap;
use pbc_contract_common::zk::{SecretVarId, ZkInputDef, ZkState, ZkStateChange};

use read_write_rpc_derive::ReadWriteRPC;
use pbc_zk::{Sbi8, SecretBinary};
use read_write_state_derive::ReadWriteState;

use crate::zk_compute::RandomnessInput;
use pbc_traits::ReadRPC;
use pbc_contract_common::shortname::Shortname;
use pbc_contract_common::context::CallbackContext;


/// Metadata information associated with each individual variable.
#[derive(ReadWriteState, ReadWriteRPC, Debug)]
#[repr(u8)]
pub enum SecretVarType {
    #[discriminant(0)]
    Randomness {},
    #[discriminant(1)]
    FlipResult {player: Address},
}

/// Player choices: Heads or Tails
#[derive(ReadWriteState, ReadWriteRPC, Debug, PartialEq, Copy, Clone, CreateTypeSpec)]
#[repr(u8)]
pub enum PlayerChoice {
    #[discriminant(0)]
    Heads {},
    #[discriminant(1)]
    Tails {},
}

/// Struct to hold player bets
#[derive(ReadWriteState, ReadWriteRPC, Debug, Clone, CreateTypeSpec)] 
pub struct PlayerBet {
    pub amount: u64,
    pub choice: PlayerChoice,
}

/// The state of the coin flip game, now supporting multiple players.
#[state]
pub struct CoinFlipState {
    player_bets: SortedVecMap<Address, PlayerBet>, 
    flip_results: SortedVecMap<Address, bool>, 
    winners: SortedVecMap<Address, Address>,
    user_balances: SortedVecMap<Address, u64>,
    game_phases: SortedVecMap<Address, GamePhase>,
    token_address: Address, // New field to store the token contract address
}

#[allow(dead_code)]
impl CoinFlipState {
    /// Check if the game is finished for the given player.
    fn is_game_finished(&self, player: &Address) -> bool {
        self.flip_results.contains_key(player)
    }

    /// Get the winner of the game for a given player.
    fn get_winner(&self, player: &Address) -> Option<Address> {
        self.winners.get(player).cloned()
    }

    /// Adjust the balance of a given user.
    fn adjust_balance(&mut self, user: Address, amount: u64) {
        if let Some(balance) = self.user_balances.get_mut(&user) {
            *balance += amount;
        } else {
            self.user_balances.insert(user, amount);
        }
    }
}

#[derive(CreateTypeSpec, SecretBinary)]
pub struct RandomContribution {
    result: Sbi8,
}

#[derive(ReadWriteRPC, ReadWriteState, CreateTypeSpec, Debug, PartialEq, Copy, Clone)]
pub enum GamePhase {
    #[discriminant(0)]
    Start {},
    #[discriminant(1)]
    PlaceBets {},
    #[discriminant(2)]
    FlipCoin {},
    #[discriminant(3)]
    Done {},
}

/// Initialize a new coin flip game.
#[init(zk = true)]
pub fn initialize(
    context: ContractContext,
    zk_state: ZkState<SecretVarType>,
    token_address: Address,  // <-- Add token_address as a parameter
) -> (CoinFlipState, Vec<EventGroup>) {
    let state = CoinFlipState {
        player_bets: SortedVecMap::new(),
        flip_results: SortedVecMap::new(),
        winners: SortedVecMap::new(),
        user_balances: SortedVecMap::new(),
        game_phases: SortedVecMap::new(),
        token_address, // Store the token address in the state
    };

    (state, vec![])
}

/// Start the game, place the bet, and choose Heads or Tails for multiple players.
/// Before starting, check if the player left the game in an inconsistent state and reset it to `Start` if needed.
#[action(shortname = 0x01, zk = true)]
pub fn start_game_and_place_bet(
    context: ContractContext,
    mut state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
    bet_amount: u64,
    choice: PlayerChoice,
) -> (CoinFlipState, Vec<EventGroup>, Vec<ZkStateChange>) {
    // Check the current phase of the player
    let player_phase = state
        .game_phases
        .get(&context.sender)
        .cloned()
        .unwrap_or(GamePhase::Start {});

    if let GamePhase::Start {} = player_phase {
        // Player is in the Start phase, no need to reset.
    } else {
        // Reset the player's state if the game was left in an inconsistent phase
        state.player_bets.remove(&context.sender);
        state.flip_results.remove(&context.sender);
        state.winners.remove(&context.sender);
        state.game_phases.insert(context.sender, GamePhase::Start {}); // Set phase to Start
    }

    // Ensure that the game is now in the `Start` phase before placing a bet
    let player_phase = state
        .game_phases
        .get(&context.sender)
        .cloned()
        .unwrap_or(GamePhase::Start {});
    assert_eq!(
        player_phase,
        GamePhase::Start {},
        "The game must be in the Start phase to place a bet."
    );

    // **Place the bet:**
    let player_bet = PlayerBet {
        amount: bet_amount,
        choice,
    };
    state.player_bets.insert(context.sender, player_bet);

    // **Transfer tokens before proceeding**:
    // Initiating token transfer and registering a callback
    let mut event_group = EventGroup::builder();

    // `transfer_from` call for the token contract
    event_group
        .call(state.token_address, Shortname::from_u32(0x03)) // Assuming shortname for `transfer_from`
        .argument(context.sender) // 'from' (sender of the coin flip contract)
        .argument(context.contract_address) // 'to' (receiver = this contract)
        .argument(bet_amount as u128) // amount to transfer
        .done();

            // Registering a callback to proceed only if the transfer is successful
        event_group
        // Using the constructor or method provided for ShortnameCallback (replace `new` with the actual method if different)
        .with_callback(pbc_contract_common::address::ShortnameCallback::new(Shortname::from_u32(0x01))) 
        .with_cost(1000)
        .argument(context.sender)
        .done();

    // Returning the event group and leaving the game in the current phase (Start) until callback
    (state, vec![event_group.build()], vec![])
}

/// Callback action to be triggered when the token transfer is successful.
#[callback(shortname = 0x01, zk = true)]
pub fn transfer_success_callback(
    context: ContractContext,
    callback_ctx: CallbackContext,
    mut state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
    player: Address,
) -> (CoinFlipState, Vec<EventGroup>, Vec<ZkStateChange>) {
    // Check if the transfer succeeded using the callback context
    assert!(
        callback_ctx.results[0].succeeded,
        "Token transfer failed, cannot proceed to the next phase."
    );

    // Now move the player to the next phase after a successful transfer
    state.game_phases.insert(player, GamePhase::FlipCoin {}); // Move the player to the next phase
    
    (state, vec![], vec![])
}


/// Add randomness for the coin flip for a specific player.
#[zk_on_secret_input(shortname = 0x40, secret_type = "RandomContribution")]
pub fn add_randomness_to_flip(
    context: ContractContext,
    state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
) -> (
    CoinFlipState,
    Vec<EventGroup>,
    ZkInputDef<SecretVarType, RandomContribution>,
) {
    let player_phase = state
        .game_phases
        .get(&context.sender)
        .cloned()
        .unwrap_or(GamePhase::Start {});
    assert_eq!(
        player_phase,
        GamePhase::FlipCoin {},
        "Must be in the FlipCoin phase to input secret randomness."
    );

    let input_def = ZkInputDef::with_metadata(
        Some(SHORTNAME_INPUTTED_VARIABLE),
        SecretVarType::Randomness {},
    );

    (state, vec![], input_def)
}

/// Automatically called when a variable is confirmed on chain.
#[zk_on_variable_inputted(shortname = 0x01)]
fn inputted_variable(
    context: ContractContext,
    mut state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
    variable_id: SecretVarId,
) -> CoinFlipState {
    state
}

/// Start the computation to compute the coin flip result for a specific player.
#[action(shortname = 0x03, zk = true)]
pub fn flip_coin(
    context: ContractContext,
    state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
) -> (CoinFlipState, Vec<EventGroup>, Vec<ZkStateChange>) {
    let player_phase = state
        .game_phases
        .get(&context.sender)
        .cloned()
        .unwrap_or(GamePhase::Start {});
    assert_eq!(
        player_phase,
        GamePhase::FlipCoin {},
        "The coin can only be flipped in the FlipCoin phase"
    );

    (
        state,
        vec![],
        vec![zk_compute::compute_coin_flip_start(
            Some(SHORTNAME_FLIP_COMPUTE_COMPLETE),
            &SecretVarType::FlipResult {player: context.sender},
        )],
    )
}



/// Automaticalladjust_balancey called when the coin flip computation is completed.
#[zk_on_compute_complete(shortname = 0x01)]
fn flip_compute_complete(
    context: ContractContext,
    state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
    output_variables: Vec<SecretVarId>,
) -> (CoinFlipState, Vec<EventGroup>, Vec<ZkStateChange>) {
    (
        state,
        vec![],
        vec![ZkStateChange::OpenVariables {
            variables: output_variables,
        }],
    )
}

/// Automatically called when the flip result variable is opened for a player.
#[zk_on_variables_opened]
fn open_flip_result_variable(
    context: ContractContext,
    mut state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
    opened_variables: Vec<SecretVarId>,
) -> (CoinFlipState, Vec<EventGroup>, Vec<ZkStateChange>) {
    assert_eq!(
        opened_variables.len(),
        1,
        "Unexpected number of output variables"
    );

    let opened_variable = zk_state
        .get_variable(*opened_variables.first().unwrap())
        .unwrap();

        if let SecretVarType::FlipResult {player} = opened_variable.metadata {
        if let Some(data) = opened_variable.data {
            let randomness_input = RandomnessInput {
                result: Sbi8::from(data[0] as i8),
            };

            let flip_result = zk_compute::parse_compute_output(randomness_input);  // true = heads, false = tails

            // Insert the result into the state
            state.flip_results.insert(player, flip_result);

            // **Change:** Ensure the game phase transitions to 'Done' for the player who started the game only
            state.game_phases.insert(player, GamePhase::Done {});

            // Determine the winner based on the player's choice and the flip result
            if let Some(player_bet) = state.player_bets.get(&player) {
                if (player_bet.choice == PlayerChoice::Heads {} && flip_result) ||
                   (player_bet.choice == PlayerChoice::Tails {} && !flip_result) {
                    state.winners.insert(player, player); // Player wins
                } else {
                    state.winners.insert(player, context.contract_address); // Main contract wins
                }
            }

            // **Change:** No phase update for the winner, keep it only for the player who started the game.
        } else {
            panic!("Expected data in the opened variable, but found None.");
        }
    }

    (state, vec![], vec![])
}

/// Payout the winner for a specific player.
#[action(shortname = 0x04, zk = true)]
pub fn payout_winner(
    context: ContractContext,
    mut state: CoinFlipState,
    zk_state: ZkState<SecretVarType>,
) -> (CoinFlipState, Vec<EventGroup>, Vec<ZkStateChange>) {
    let player_phase = state
        .game_phases
        .get(&context.sender)
        .cloned()
        .unwrap_or(GamePhase::Start {});
    assert_eq!(
        player_phase,
        GamePhase::Done {},
        "Payout can only occur after the game has completed."
    );

    if let Some(winner) = state.get_winner(&context.sender) {
        // If the winner is the player themselves
        if winner == context.sender {
            if let Some(bet) = state.player_bets.get(&context.sender) {
                // Calculate the winnings (double the bet)
                let winnings = bet.amount * 2;

                // Adjust player's balance
                state.adjust_balance(context.sender, winnings);

                // Create an event group to transfer tokens to the winner
                let mut event_group = EventGroup::builder();

                // Call the token contract's `transfer` method
                event_group
                    .call(state.token_address, Shortname::from_u32(0x01)) // Assuming shortname for `transfer`
                    .argument(context.sender) // 'to' (the winning player)
                    .argument(winnings as u128) // amount to transfer
                    .done();

                // // After the payout, reset the player's state
                // state.player_bets.remove(&context.sender);
                // state.flip_results.remove(&context.sender);
                // state.winners.remove(&context.sender);
                // state.game_phases.insert(context.sender, GamePhase::Start {}); // Reset phase to Start

                return (state, vec![event_group.build()], vec![]);
            }
        }
    }

    // If no payout is needed or winner is not the player, return empty event group
    (state, vec![], vec![])
}
