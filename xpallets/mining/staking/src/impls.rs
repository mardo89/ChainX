use super::*;
use codec::Encode;
use sp_arithmetic::traits::BaseArithmetic;
use sp_core::crypto::UncheckedFrom;
use sp_runtime::{traits::Hash, Perbill};
use sp_staking::offence::{OffenceDetails, OnOffenceHandler};
use xp_mining_common::{
    generic_weight_factors, BaseMiningWeight, Claim, ComputeMiningWeight, WeightFactors, WeightType,
};
use xp_mining_staking::SessionIndex;

impl<Balance, BlockNumber> BaseMiningWeight<Balance, BlockNumber>
    for ValidatorLedger<Balance, BlockNumber>
where
    Balance: Default + BaseArithmetic + Copy,
    BlockNumber: Default + BaseArithmetic + Copy,
{
    fn amount(&self) -> Balance {
        self.total
    }

    fn set_amount(&mut self, new: Balance) {
        self.total = new;
    }

    fn last_acum_weight(&self) -> WeightType {
        self.last_total_vote_weight
    }

    fn set_last_acum_weight(&mut self, latest_vote_weight: WeightType) {
        self.last_total_vote_weight = latest_vote_weight;
    }

    fn last_acum_weight_update(&self) -> BlockNumber {
        self.last_total_vote_weight_update
    }

    fn set_last_acum_weight_update(&mut self, current_block: BlockNumber) {
        self.last_total_vote_weight_update = current_block;
    }
}

impl<Balance, BlockNumber> BaseMiningWeight<Balance, BlockNumber>
    for NominatorLedger<Balance, BlockNumber>
where
    Balance: Default + BaseArithmetic + Copy,
    BlockNumber: Default + BaseArithmetic + Copy,
{
    fn amount(&self) -> Balance {
        self.nomination
    }

    fn set_amount(&mut self, new: Balance) {
        self.nomination = new;
    }

    fn last_acum_weight(&self) -> WeightType {
        self.last_vote_weight
    }

    fn set_last_acum_weight(&mut self, latest_vote_weight: WeightType) {
        self.last_vote_weight = latest_vote_weight;
    }

    fn last_acum_weight_update(&self) -> BlockNumber {
        self.last_vote_weight_update
    }

    fn set_last_acum_weight_update(&mut self, current_block: BlockNumber) {
        self.last_vote_weight_update = current_block;
    }
}

impl<T: Trait> ComputeMiningWeight<T::AccountId, T::BlockNumber> for Module<T> {
    type Claimee = T::AccountId;
    type Error = Error<T>;

    fn claimer_weight_factors(
        who: &T::AccountId,
        target: &Self::Claimee,
        current_block: T::BlockNumber,
    ) -> WeightFactors {
        let claimer_ledger = Nominations::<T>::get(who, target);
        generic_weight_factors::<T::Balance, T::BlockNumber, _>(claimer_ledger, current_block)
    }

    fn claimee_weight_factors(
        target: &Self::Claimee,
        current_block: T::BlockNumber,
    ) -> WeightFactors {
        let claimee_ledger = ValidatorLedgers::<T>::get(target);
        generic_weight_factors::<T::Balance, T::BlockNumber, _>(claimee_ledger, current_block)
    }
}

/// Computes the dividend according to the ratio of source_vote_weight/target_vote_weight.
///
/// dividend = source_vote_weight/target_vote_weight * balance_of(claimee_reward_pot)
pub fn compute_dividend<T: Trait>(
    source_vote_weight: WeightType,
    target_vote_weight: WeightType,
    claimee_reward_pot: &T::AccountId,
) -> T::Balance {
    let total_reward_pot = xpallet_assets::Module::<T>::pcx_free_balance(&claimee_reward_pot);
    match source_vote_weight.checked_mul(total_reward_pot.saturated_into()) {
        Some(x) => ((x / target_vote_weight) as u64).saturated_into(),
        None => panic!("source_vote_weight * total_reward_pot overflow, this should not happen"),
    }
}

impl<T: Trait> Module<T> {
    fn allocate_dividend(
        claimer: &T::AccountId,
        pot_account: &T::AccountId,
        dividend: T::Balance,
    ) -> Result<(), AssetErr> {
        xpallet_assets::Module::<T>::pcx_move_free_balance(pot_account, claimer, dividend)
    }

    /// Calculates the new amount given the origin amount and delta
    fn apply_delta(origin: T::Balance, delta: Delta<T::Balance>) -> T::Balance {
        match delta {
            Delta::Add(v) => origin + v,
            Delta::Sub(v) => origin - v,
            Delta::Zero => origin,
        }
    }

    /// Actually update the nominator vote weight given the new vote weight, block number and amount delta.
    pub(crate) fn set_nominator_vote_weight(
        nominator: &T::AccountId,
        validator: &T::AccountId,
        new_weight: WeightType,
        current_block: T::BlockNumber,
        delta: Delta<T::Balance>,
    ) {
        Nominations::<T>::mutate(nominator, validator, |claimer_ledger| {
            claimer_ledger.nomination = Self::apply_delta(claimer_ledger.nomination, delta);
            claimer_ledger.last_vote_weight = new_weight;
            claimer_ledger.last_vote_weight_update = current_block;
        });
    }

    ///
    pub(crate) fn set_validator_vote_weight(
        validator: &T::AccountId,
        new_weight: WeightType,
        current_block: T::BlockNumber,
        delta: Delta<T::Balance>,
    ) {
        ValidatorLedgers::<T>::mutate(validator, |validator_ledger| {
            validator_ledger.total = Self::apply_delta(validator_ledger.total, delta);
            validator_ledger.last_total_vote_weight = new_weight;
            validator_ledger.last_total_vote_weight_update = current_block;
        });
    }

    fn update_claimer_vote_weight_on_claim(
        claimer: &T::AccountId,
        target: &T::AccountId,
        current_block: T::BlockNumber,
    ) {
        Self::set_nominator_vote_weight(claimer, target, 0, current_block, Delta::Zero);
    }

    fn update_claimee_vote_weight_on_claim(
        claimee: &T::AccountId,
        new_vote_weight: WeightType,
        current_block: T::BlockNumber,
    ) {
        Self::set_validator_vote_weight(claimee, new_vote_weight, current_block, Delta::Zero);
    }
}

impl<T: Trait> Claim<T::AccountId> for Module<T> {
    type Claimee = T::AccountId;
    type Error = Error<T>;

    fn claim(claimer: &T::AccountId, claimee: &Self::Claimee) -> Result<(), Self::Error> {
        let current_block = <frame_system::Module<T>>::block_number();

        let (source_weight, target_weight) = <Self as ComputeMiningWeight<
            T::AccountId,
            T::BlockNumber,
        >>::settle_weight_on_claim(
            claimer, claimee, current_block
        )?;

        let claimee_pot = Self::reward_pot_for(claimee);

        let dividend = compute_dividend::<T>(source_weight, target_weight, &claimee_pot);

        Self::allocate_dividend(claimer, &claimee_pot, dividend)?;

        Self::deposit_event(RawEvent::Claim(claimer.clone(), claimee.clone(), dividend));

        let new_target_weight = target_weight - source_weight;

        Self::update_claimer_vote_weight_on_claim(claimer, claimee, current_block);
        Self::update_claimee_vote_weight_on_claim(claimee, new_target_weight, current_block);

        Ok(())
    }
}

/// In this implementation `new_session(session)` must be called before `end_session(session-1)`
/// i.e. the new session must be planned before the ending of the previous session.
///
/// Once the first new_session is planned, all session must start and then end in order, though
/// some session can lag in between the newest session planned and the latest session started.
impl<T: Trait> pallet_session::SessionManager<T::AccountId> for Module<T> {
    fn new_session(new_index: SessionIndex) -> Option<Vec<T::AccountId>> {
        Self::new_session(new_index)
    }
    fn start_session(start_index: SessionIndex) {
        Self::start_session(start_index)
    }
    fn end_session(end_index: SessionIndex) {
        Self::end_session(end_index)
    }
}

type OnOffenceRes = u64;
/// Validator ID that reported this offence.
type Reporter<T> = <T as frame_system::Trait>::AccountId;

/// Substrate:
/// A tuple of the validator's ID and their full identification.
/// pub type IdentificationTuple<T> = (<T as crate::Trait>::ValidatorId, <T as Trait>::FullIdentification);
/// ChainX:
/// We do not have the FullIdentification info, but the reward pot.
pub type IdentificationTuple<T> = (
    <T as frame_system::Trait>::AccountId,
    <T as frame_system::Trait>::AccountId,
);

/// Stable ID of a validator.
type Offender<T> = IdentificationTuple<T>;

/// This is intended to be used with `FilterHistoricalOffences` in Substrate/Staking.
/// In ChainX, we always apply the slash immediately, no deferred slash.
impl<T: Trait> OnOffenceHandler<Reporter<T>, IdentificationTuple<T>, OnOffenceRes> for Module<T>
where
    T: pallet_session::Trait<ValidatorId = <T as frame_system::Trait>::AccountId>,
    T::SessionHandler: pallet_session::SessionHandler<<T as frame_system::Trait>::AccountId>,
    T::SessionManager: pallet_session::SessionManager<<T as frame_system::Trait>::AccountId>,
    T::ValidatorIdOf: Convert<
        <T as frame_system::Trait>::AccountId,
        Option<<T as frame_system::Trait>::AccountId>,
    >,
{
    /// Always returns Ok(_), i.e., notify the Offences module that the slashing will
    /// be applied immediately, no deferred slashing.
    ///
    /// `on_offence()` is triggered inside `on_before_session_ending()`.
    fn on_offence(
        offenders: &[OffenceDetails<Reporter<T>, Offender<T>>],
        slash_fraction: &[Perbill],
        _slash_session: SessionIndex,
    ) -> Result<OnOffenceRes, ()> {
        let offenders_in_session = offenders
            .into_iter()
            .zip(slash_fraction)
            .map(|(details, slash_fraction)| {
                // reporters are actually none at the moment.
                let (offender, _reporters) = &details.offender;
                (offender.clone(), slash_fraction.clone())
            })
            .collect::<Vec<_>>();

        // Note the offenders in current session.
        <OffendersInSession<T>>::put(offenders_in_session);

        Ok(0)
    }

    fn can_report() -> bool {
        true
    }
}

/// Simple validator reward pot account determiner.
///
/// Formula: `blake2_256(blake2_256(validator_pubkey) + blake2_256(registered_at))`
pub struct SimpleValidatorRewardPotAccountDeterminer<T: Trait>(sp_std::marker::PhantomData<T>);

impl<T: Trait> xp_mining_common::RewardPotAccountFor<T::AccountId, T::AccountId>
    for SimpleValidatorRewardPotAccountDeterminer<T>
where
    T::AccountId: UncheckedFrom<T::Hash> + AsRef<[u8]>,
{
    fn reward_pot_account_for(validator: &T::AccountId) -> T::AccountId {
        let validator_hash = <T as frame_system::Trait>::Hashing::hash(validator.as_ref());
        let registered_at: T::BlockNumber = Validators::<T>::get(validator).registered_at;
        let registered_at_hash =
            <T as frame_system::Trait>::Hashing::hash(registered_at.encode().as_ref());

        let validator_slice = validator_hash.as_ref();
        let registered_at_slice = registered_at_hash.as_ref();

        let mut buf = Vec::with_capacity(validator_slice.len() + registered_at_slice.len());
        buf.extend_from_slice(validator_slice);
        buf.extend_from_slice(registered_at_slice);

        UncheckedFrom::unchecked_from(T::Hashing::hash(&buf[..]))
    }
}
