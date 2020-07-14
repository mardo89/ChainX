use super::*;

impl<T: Trait> Module<T> {
    fn new_session(session_index: SessionIndex) -> Option<Vec<T::AccountId>> {
        // TODO: the whole flow of session changes?
        //
        // Only the active validators can be rewarded.
        let staking_reward = Self::distribute_session_reward(session_index);

        let force_chilled = Self::slash_offenders_in_session(staking_reward);

        if force_chilled > 0 {
            // Force a new era if some offender's reward pot has been wholly slashed.
            Self::ensure_new_era();
        }

        debug!(
            "[new_session]session_index:{:?}, current_era:{:?}",
            session_index,
            Self::current_era()
        );

        // FIXME: force new era when some validator's reward pot has been all slashed.
        if let Some(current_era) = Self::current_era() {
            // Initial era has been set.

            let current_era_start_session_index = Self::eras_start_session_index(current_era)
                .unwrap_or_else(|| {
                    frame_support::print("Error: start_session_index must be set for current_era");
                    0
                });

            let era_length = session_index
                .checked_sub(current_era_start_session_index)
                .unwrap_or(0); // Must never happen.

            let ideal_era_length = Self::sessions_per_era().saturated_into::<SessionIndex>();

            match ForceEra::get() {
                Forcing::ForceNew => ForceEra::kill(),
                Forcing::ForceAlways => (),
                Forcing::NotForcing if era_length >= ideal_era_length => (),
                _ => {
                    // Either `ForceNone`, or `NotForcing && era_length < T::SessionsPerEra::get()`.
                    if era_length + 1 == ideal_era_length {
                        IsCurrentSessionFinal::put(true);
                    } else if era_length >= ideal_era_length {
                        // Should only happen when we are ready to trigger an era but we have ForceNone,
                        // otherwise previous arm would short circuit.
                        // FIXME: figure out this
                        // Self::close_election_window();
                    }
                    return None;
                }
            }

            // new era.
            Self::new_era(session_index)
        } else {
            // Set initial era
            Self::new_era(session_index)
        }
    }

    /// Start a session potentially starting an era.
    fn start_session(start_session: SessionIndex) {
        let next_active_era = Self::active_era().map(|e| e.index + 1).unwrap_or(0);
        debug!(
            "[start_session]:start_session:{:?}, next_active_era:{:?}",
            start_session, next_active_era
        );
        if let Some(next_active_era_start_session_index) =
            Self::eras_start_session_index(next_active_era)
        {
            if next_active_era_start_session_index == start_session {
                Self::start_era(start_session);
            } else if next_active_era_start_session_index < start_session {
                // This arm should never happen, but better handle it than to stall the
                // staking pallet.
                frame_support::print("Warning: A session appears to have been skipped.");
                Self::start_era(start_session);
            }
        }
    }

    /// End a session potentially ending an era.
    fn end_session(session_index: SessionIndex) {
        if let Some(active_era) = Self::active_era() {
            if let Some(next_active_era_start_session_index) =
                Self::eras_start_session_index(active_era.index + 1)
            {
                if next_active_era_start_session_index == session_index + 1 {
                    Self::end_era(active_era, session_index);
                }
            }
        }
    }

    /// * Increment `active_era.index`,
    /// * reset `active_era.start`,
    /// * update `BondedEras` and apply slashes.
    fn start_era(_start_session: SessionIndex) {
        let active_era = ActiveEra::mutate(|active_era| {
            let new_index = active_era.as_ref().map(|info| info.index + 1).unwrap_or(0);
            *active_era = Some(ActiveEraInfo {
                index: new_index,
                // Set new active era start in next `on_finalize`. To guarantee usage of `Time`
                start: None,
            });
            new_index
        });
    }

    /// Compute payout for era.
    fn end_era(active_era: ActiveEraInfo, session_index: SessionIndex) {
        debug!(
            "[end_era]active_era:{:?}, session_index:{:?}",
            active_era, session_index
        );
    }
}
