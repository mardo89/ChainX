// Copyright 2018-2019 Chainpool.

use super::*;
use rstd::convert::TryInto;
use xaccounts::IntentionJackpotAccountIdFor;

mod proposal09;

impl<T: Trait> Module<T> {
    /// Get the reward for the session, assuming it ends with this block.
    fn this_session_reward() -> T::Balance {
        let current_index = <xsession::Module<T>>::current_index().saturated_into::<u64>();
        let reward = Self::initial_reward().into()
            / u64::from(u32::pow(2, (current_index / SESSIONS_PER_ROUND) as u32));
        reward.into()
    }

    /// Issue new fresh PCX.
    #[inline]
    fn mint(receiver: &T::AccountId, value: T::Balance) {
        let _ = <xassets::Module<T>>::pcx_issue(receiver, value);
    }

    /// Reward a given (potential) validator by a specific amount.
    /// Add the reward to their balance, and their jackpot, pro-rata.
    fn reward(who: &T::AccountId, reward: T::Balance) {
        // Validator themselves only gain 10%, the rest 90% goes to its jackpot.
        let off_the_table = (reward.into() / 10).into();
        Self::mint(who, off_the_table);
        debug!("[reward]issue to {:?}: {:?}", who!(who), off_the_table);

        // Issue the rest 90% to validator's jackpot.
        let to_jackpot = reward - off_the_table;
        let jackpot_addr = T::DetermineIntentionJackpotAccountId::accountid_for_unsafe(who);
        Self::mint(&jackpot_addr, to_jackpot);
        debug!(
            "[reward] issue to {:?}'s jackpot: {:?}",
            who!(who),
            to_jackpot
        );
    }

    /// Reward the intention and slash the validators that went offline in last session.
    ///
    /// If the slashed validator can't afford that penalty, it will be
    /// removed from the validator list.
    #[inline]
    fn reward_active_intention_and_try_slash(
        intention: &T::AccountId,
        reward: T::Balance,
        validators: &mut Vec<T::AccountId>,
    ) {
        Self::reward(intention, reward);
        // It the intention was an offline validator, we should enforce a slash.
        if <MissedOfPerSession<T>>::exists(intention) {
            Self::slash_active_offline_validator(intention, reward, validators);
        }
    }

    #[inline]
    fn generic_calculate_by_proportion<S: Into<u128>>(
        total_reward: T::Balance,
        mine: S,
        total: S,
    ) -> T::Balance {
        let mine: u128 = mine.into();
        let total: u128 = total.into();

        match mine.checked_mul(u128::from(total_reward.into())) {
            Some(x) => {
                let r = x / total;
                assert!(
                    r < u128::from(u64::max_value()),
                    "reward of per intention definitely less than u64::max_value()"
                );
                (r as u64).into()
            }
            None => panic!("stake * session_reward overflow!"),
        }
    }

    /// Calculate the individual reward according to the proportion and total reward.
    fn calculate_reward_by_stake(
        total_reward: T::Balance,
        my_stake: T::Balance,
        total_stake: T::Balance,
    ) -> T::Balance {
        let mine: u64 = my_stake.into();
        let total: u64 = total_stake.into();
        Self::generic_calculate_by_proportion(total_reward, mine, total)
    }

    /// Calculate the individual reward according to the mining power of cross chain assets.
    fn multiply_by_mining_power(
        total_reward: T::Balance,
        my_power: u128,
        total_mining_power: u128,
    ) -> T::Balance {
        Self::generic_calculate_by_proportion(total_reward, my_power, total_mining_power)
    }

    // This is guarantee not to overflow on whatever values.
    // `num` must be inferior to `den` otherwise it will be reduce to `den`.
    pub fn multiply_by_rational(value: u64, num: u32, den: u32) -> u64 {
        let num = num.min(den);

        let result_divisor_part: u64 = value / u64::from(den) * u64::from(num);

        let result_remainder_part: u64 = {
            let rem: u64 = value % u64::from(den);

            // Fits into u32 because den is u32 and remainder < den
            let rem_u32 = rem.try_into().unwrap_or(u32::max_value());

            // Multiplication fits into u64 as both term are u32
            let rem_part = u64::from(rem_u32) * u64::from(num) / u64::from(den);

            // Result fits into u32 as num < total_points
            (rem_part as u32).into()
        };

        result_divisor_part + result_remainder_part
    }

    /// Collect all the active intentions as well as their total nomination.
    ///
    /// Only these active intentions are able to be rewarded on each new session,
    /// the inactive ones earns nothing.
    fn get_active_intentions_info() -> impl Iterator<Item = (T::AccountId, T::Balance)> {
        Self::intention_set()
            .into_iter()
            .filter(Self::is_active)
            .map(|id| {
                let total_nomination = Self::total_nomination_of(&id);
                (id, total_nomination)
            })
    }

    /// In the first round, 20% reward of each session goes to the team.
    fn try_fund_team(this_session_reward: T::Balance) -> T::Balance {
        let current_index = <xsession::Module<T>>::current_index().saturated_into::<u64>();

        if current_index < SESSIONS_PER_ROUND {
            let to_team = (this_session_reward.into() / 5).into();
            debug!("[try_fund_team] issue to the team: {:?}", to_team);
            Self::mint(&xaccounts::Module::<T>::team_account(), to_team);
            this_session_reward - to_team
        } else {
            this_session_reward
        }
    }

    /// Distribute the session reward for (psedu-)intentions.
    pub(super) fn distribute_session_reward(validators: &mut Vec<T::AccountId>) {
        let this_session_reward = Self::this_session_reward();
        let session_reward = Self::try_fund_team(this_session_reward);

        Self::distribute_session_reward_impl_09(session_reward, validators);
    }
}
