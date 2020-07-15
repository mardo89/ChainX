use super::*;

impl<T: Trait> Module<T> {
    /// Actually slash the account being punished, all slashed balance will go to the treasury.
    #[inline]
    fn apply_slash(reward_pot: &T::AccountId, treasury_account: &T::AccountId, value: T::Balance) {
        let _ =
            <xpallet_assets::Module<T>>::pcx_move_free_balance(reward_pot, treasury_account, value);
    }

    fn reward_per_block(staking_reward: T::Balance, validator_count: usize) -> u128 {
        let session_length = T::SessionDuration::get();
        let per_reward = staking_reward.saturated_into::<u128>()
            * validator_count.saturated_into::<u128>()
            / session_length.saturated_into::<u128>();
        per_reward
    }

    /// Returns Ok(_) if the offender's reward pot has enough balance to cover the slash,
    /// otherwise returns Err(_) which means the offender should be forced to be chilled
    /// and the whole reward pot will be slashed.
    fn try_slash(
        offender: &T::AccountId,
        treasury_account: &T::AccountId,
        expected_slash: T::Balance,
    ) -> Result<(), T::Balance> {
        let reward_pot = Self::reward_pot_for(offender);
        let reward_pot_balance = <xpallet_assets::Module<T>>::pcx_free_balance(&reward_pot);

        if expected_slash <= reward_pot_balance {
            Self::apply_slash(&reward_pot, treasury_account, expected_slash);
            Ok(())
        } else {
            Self::apply_slash(&reward_pot, treasury_account, reward_pot_balance);
            Err(reward_pot_balance)
        }
    }

    // TODO: optimize with try_slash()
    fn expected_slash_of(offender: &T::AccountId, slash_fraction: Perbill) -> T::Balance {
        let reward_pot = Self::reward_pot_for(offender);
        let reward_pot_balance = <xpallet_assets::Module<T>>::pcx_free_balance(&reward_pot);
        // FIXME: apply a multiplier
        slash_fraction * reward_pot_balance
    }

    /// Slash the offenders in `end_session()`.
    pub(crate) fn slash_offenders_in_session() -> u64 {
        let active_potential_validators = Validators::<T>::iter()
            .map(|(v, _)| v)
            .filter(Self::is_active)
            .collect::<Vec<_>>();

        let minimum_validator_count = Self::minimum_validator_count() as usize;
        let mut active_count = active_potential_validators.len();

        let offenders = OffendersInSession::<T>::take();
        let mut force_chilled = 0;

        /*
        // Find the session validators that is still active atm.
        let reward_validators = T::SessionInterface::validators()
            .into_iter()
            .filter(Self::is_active)
            .collect::<Vec<_>>();

        // FIXME: we have no idea of how many blocks an offender actually missed,
        // the slash per missing block makes less sense now.
        let reward_per_block = Self::reward_per_block(staking_reward, reward_validators.len());
        */

        // Cache the treasury_account
        let treasury_account = T::TreasuryAccount::treasury_account();

        for (offender, slash_fraction) in offenders.into_iter() {
            let expected_slash = Self::expected_slash_of(&offender, slash_fraction);

            if let Err(actual_slashed) =
                Self::try_slash(&offender, &treasury_account, expected_slash)
            {
                debug!(
                    "[slash_offenders_in_session]expected_slash:{:?}, actual_slashed:{:?}",
                    expected_slash, actual_slashed
                );
                if active_count > minimum_validator_count {
                    Self::apply_force_chilled(&offender);
                    active_count -= 1;
                    force_chilled += 1;
                    // FIXME: is it still neccessary to T::SessionInterface::disable_validator()?
                }
            }
        }

        force_chilled
    }
}
