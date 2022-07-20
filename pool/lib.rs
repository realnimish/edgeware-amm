#![cfg_attr(not(feature = "std"), no_std)]

use ink_lang as ink;
pub use pool::{Pool, PoolRef};

#[ink::contract]
mod pool {
    use erc20::Erc20Ref as Erc20;
    use ink_storage::{traits::SpreadAllocate, Mapping};

    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        /// Zero Liquidity
        ZeroLiquidity,
        /// Amount cannot be zero!
        ZeroAmount,
        /// Insufficient amount
        InsufficientAmount,
        /// Equivalent value of tokens not provided
        NonEquivalentValue,
        /// Asset value less than threshold for contribution!
        ThresholdNotReached,
        /// Share should be less than totalShare
        InvalidShare,
        /// Insufficient pool balance
        InsufficientLiquidity,
        /// Slippage tolerance exceeded
        SlippageExceeded,
        /// Returned if not enough balance to fulfill a request is available.
        InsufficientBalance,
        /// Returned if not enough allowance to fulfill a request is available.
        InsufficientAllowance,
    }

    /// Event emitted when a token transfer occurs.
    #[ink(event)]
    pub struct Transfer {
        #[ink(topic)]
        from: Option<AccountId>,
        #[ink(topic)]
        to: Option<AccountId>,
        value: u128,
    }

    /// Event emitted when an approval occurs that `spender` is allowed to withdraw
    /// up to the amount of `value` tokens from `owner`.
    #[ink(event)]
    pub struct Approval {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        spender: AccountId,
        value: u128,
    }

    #[ink(storage)]
    #[derive(SpreadAllocate)]
    pub struct Pool {
        token1: AccountId,
        token2: AccountId,
        total_token1: u128,
        total_token2: u128,
        total_shares: u128,
        shares: Mapping<AccountId, u128>,
        allowances: Mapping<(AccountId, AccountId), u128>,
        fees: u16,
    }

    fn erc20(addr: AccountId) -> Erc20 {
        ink_env::call::FromAccountId::from_account_id(addr)
    }

    type Result<T> = core::result::Result<T, Error>;

    impl Pool {
        #[ink(constructor)]
        pub fn new(token1: AccountId, token2: AccountId, fees: u16) -> Self {
            ink_lang::utils::initialize_contract(|contract| {
                Self::new_init(contract, token1, token2, fees)
            })
        }

        fn new_init(&mut self, token1: AccountId, token2: AccountId, fees: u16) {
            self.token1 = token1;
            self.token2 = token2;
            self.total_token1 = 0;
            self.total_token2 = 0;
            self.total_shares = 0;
            self.fees = fees;
        }
    }

    impl Pool {
        /// Returns amount of Token1 required when providing liquidity with _amount_token2 quantity of Token2
        #[ink(message)]
        pub fn get_equivalent_token1_estimate_given_token2(
            &self,
            _amount_token2: u128,
        ) -> Result<u128> {
            self.active_pool()?;
            Ok(self.total_token1 * _amount_token2 / self.total_token2)
        }

        /// Returns amount of Token2 required when providing liquidity with _amount_token1 quantity of Token1
        #[ink(message)]
        pub fn get_equivalent_token2_estimate_given_token1(
            &self,
            _amount_token1: u128,
        ) -> Result<u128> {
            self.active_pool()?;
            Ok(self.total_token2 * _amount_token1 / self.total_token1)
        }

        /// Adding new liquidity in the pool
        /// Returns the amount of share issued for locking given assets
        #[ink(message)]
        pub fn add_liquidity(
            &mut self,
            _amount_token1: u128,
            _amount_token2: u128,
        ) -> Result<u128> {
            let caller = self.env().caller();

            let share: u128;
            if self.total_shares == 0 {
                // Genesis liquidity is issued 100 Shares
                share = 100 * u128::pow(10, self.decimals() as u32);
            } else {
                let share1 = self.total_shares * _amount_token1 / self.total_token1;
                let share2 = self.total_shares * _amount_token2 / self.total_token2;

                if share1 != share2 {
                    return Err(Error::NonEquivalentValue);
                }
                share = share1;
            }

            if share == 0 {
                return Err(Error::ThresholdNotReached);
            }

            let me = self.env().account_id();
            erc20(self.token1)
                .transfer_from(caller, me, _amount_token1)
                .expect("Failed to receive token");

            erc20(self.token2)
                .transfer_from(caller, me, _amount_token2)
                .expect("Failed to receive token");

            self.total_token1 += _amount_token1;
            assert_eq!(erc20(self.token1).balance_of(me), self.total_token1);

            self.total_token2 += _amount_token2;
            assert_eq!(erc20(self.token2).balance_of(me), self.total_token2);

            self.total_shares += share;

            let new_share = self.shares.get(caller).map(|v| v + share).unwrap_or(share);
            self.shares.insert(caller, &new_share);

            Ok(share)
        }

        /// Returns the estimate of Token1 & Token2 that will be released on burning given _share
        #[ink(message)]
        pub fn get_withdraw_estimate(&self, _share: u128) -> Result<(u128, u128)> {
            self.active_pool()?;
            if _share > self.total_shares {
                return Err(Error::InvalidShare);
            }

            let amount_token1 = _share * self.total_token1 / self.total_shares;
            let amount_token2 = _share * self.total_token2 / self.total_shares;
            Ok((amount_token1, amount_token2))
        }

        /// Removes liquidity from the pool and releases corresponding Token1 & Token2 to the withdrawer
        #[ink(message)]
        pub fn remove_liquidity(&mut self, _share: u128) -> Result<(u128, u128)> {
            let caller = self.env().caller();
            assert!(_share <= self.shares.get(caller).unwrap_or_default());

            let (amount_token1, amount_token2) = self.get_withdraw_estimate(_share)?;
            let new_share = self.shares.get(caller).unwrap() - _share;
            self.shares.insert(caller, &new_share);
            self.total_shares -= _share;

            self.total_token1 -= amount_token1;
            self.total_token2 -= amount_token2;

            erc20(self.token1)
                .transfer(caller, amount_token1)
                .expect("Failed to withdraw");
            erc20(self.token2)
                .transfer(caller, amount_token2)
                .expect("Failed to withdraw");

            Ok((amount_token1, amount_token2))
        }

        /// Returns the amount of Token2 that the user will get when swapping a given amount of Token1 for Token2
        #[ink(message)]
        pub fn get_swap_token1_estimate_given_token1(&self, _amount_token1: u128) -> Result<u128> {
            self.active_pool()?;
            let _amount_token1 = _amount_token1 * (1000 - self.fees) as u128 / 1000; // Adjusting the fees charged

            let token1_after = self.total_token1 + _amount_token1;
            let token2_after = self.get_k() / token1_after;
            let mut amount_token2 = self.total_token2 - token2_after;

            // To ensure that Token2's pool is not completely depleted leading to inf:0 ratio
            if amount_token2 == self.total_token2 {
                amount_token2 -= 1;
            }
            Ok(amount_token2)
        }

        /// Returns the amount of Token1 that the user should swap to get _amount_token2 in return
        #[ink(message)]
        pub fn get_swap_token1_estimate_given_token2(&self, _amount_token2: u128) -> Result<u128> {
            self.active_pool()?;
            if _amount_token2 >= self.total_token2 {
                return Err(Error::InsufficientLiquidity);
            }

            let token2_after = self.total_token2 - _amount_token2;
            let token1_after = self.get_k() / token2_after;
            let amount_token1 =
                (token1_after - self.total_token1) * 1000 / (1000 - self.fees) as u128;
            Ok(amount_token1)
        }

        /// Swaps given amount of Token1 to Token2 using algorithmic price determination
        /// Swap fails if Token2 amount is less than _min_token2
        #[ink(message)]
        pub fn swap_token1_given_token1(
            &mut self,
            _amount_token1: u128,
            _min_token2: u128,
        ) -> Result<u128> {
            let caller = self.env().caller();

            let amount_token2 = self.get_swap_token1_estimate_given_token1(_amount_token1)?;
            if amount_token2 < _min_token2 {
                return Err(Error::SlippageExceeded);
            }

            let me = self.env().account_id();
            erc20(self.token1)
                .transfer_from(caller, me, _amount_token1)
                .expect("Failed to receive token");

            self.total_token1 += _amount_token1;
            assert_eq!(erc20(self.token1).balance_of(me), self.total_token1);

            self.total_token2 -= amount_token2;
            erc20(self.token2)
                .transfer(caller, amount_token2)
                .expect("Failed to withdraw");
            Ok(amount_token2)
        }

        /// Swaps given amount of Token1 to Token2 using algorithmic price determination
        /// Swap fails if amount of Token1 required to obtain _amount_token2 exceeds _max_token1
        #[ink(message)]
        pub fn swap_token1_given_token2(
            &mut self,
            _amount_token2: u128,
            _max_token1: u128,
        ) -> Result<u128> {
            let caller = self.env().caller();
            let amount_token1 = self.get_swap_token1_estimate_given_token2(_amount_token2)?;
            if amount_token1 > _max_token1 {
                return Err(Error::SlippageExceeded);
            }

            let me = self.env().account_id();
            erc20(self.token1)
                .transfer_from(caller, me, amount_token1)
                .expect("Failed to receive token");

            self.total_token1 += amount_token1;
            assert_eq!(erc20(self.token1).balance_of(me), self.total_token1);

            self.total_token2 -= _amount_token2;
            erc20(self.token2)
                .transfer(caller, _amount_token2)
                .expect("Failed to withdraw");
            Ok(amount_token1)
        }

        /// Returns the amount of Token2 that the user will get when swapping a given amount of Token1 for Token2
        #[ink(message)]
        pub fn get_swap_token2_estimate_given_token2(&self, _amount_token2: u128) -> Result<u128> {
            self.active_pool()?;
            let _amount_token2 = _amount_token2 * (1000 - self.fees) as u128 / 1000; // Adjusting the fees charged

            let token2_after = self.total_token2 + _amount_token2;
            let token1_after = self.get_k() / token2_after;
            let mut amount_token1 = self.total_token1 - token1_after;

            // To ensure that Token1's pool is not completely depleted leading to inf:0 ratio
            if amount_token1 == self.total_token1 {
                amount_token1 -= 1;
            }
            Ok(amount_token1)
        }

        /// Returns the amount of Token2 that the user should swap to get _amount_token1 in return
        #[ink(message)]
        pub fn get_swap_token2_estimate_given_token1(&self, _amount_token1: u128) -> Result<u128> {
            self.active_pool()?;
            if _amount_token1 >= self.total_token1 {
                return Err(Error::InsufficientLiquidity);
            }

            let token1_after = self.total_token1 - _amount_token1;
            let token2_after = self.get_k() / token1_after;
            let amount_token2 =
                (token2_after - self.total_token2) * 1000 / (1000 - self.fees) as u128;
            Ok(amount_token2)
        }

        /// Swaps given amount of Token2 to Token1 using algorithmic price determination
        /// Swap fails if Token1 amount is less than _min_token1
        #[ink(message)]
        pub fn swap_token2_given_token2(
            &mut self,
            _amount_token2: u128,
            _min_token1: u128,
        ) -> Result<u128> {
            let caller = self.env().caller();

            let amount_token1 = self.get_swap_token2_estimate_given_token2(_amount_token2)?;
            if amount_token1 < _min_token1 {
                return Err(Error::SlippageExceeded);
            }

            let me = self.env().account_id();
            erc20(self.token2)
                .transfer_from(caller, me, _amount_token2)
                .expect("Failed to receive token");

            self.total_token2 += _amount_token2;
            assert_eq!(erc20(self.token2).balance_of(me), self.total_token2);

            self.total_token1 -= amount_token1;
            erc20(self.token1)
                .transfer(caller, amount_token1)
                .expect("Failed to transfer token");
            Ok(amount_token1)
        }

        /// Swaps given amount of Token2 to Token1 using algorithmic price determination
        /// Swap fails if amount of Token2 required to obtain _amount_token1 exceeds _max_token2
        #[ink(message)]
        pub fn swap_token2_given_token1(
            &mut self,
            _amount_token1: u128,
            _max_token2: u128,
        ) -> Result<u128> {
            let caller = self.env().caller();

            let amount_token2 = self.get_swap_token2_estimate_given_token1(_amount_token1)?;
            if amount_token2 > _max_token2 {
                return Err(Error::SlippageExceeded);
            }

            let me = self.env().account_id();
            erc20(self.token2)
                .transfer_from(caller, me, amount_token2)
                .expect("Failed to receive token");

            self.total_token2 += amount_token2;
            assert_eq!(erc20(self.token2).balance_of(me), self.total_token2);

            self.total_token1 -= _amount_token1;
            erc20(self.token1)
                .transfer(caller, _amount_token1)
                .expect("Failed to transfer token");
            Ok(amount_token2)
        }
    }

    impl Pool {
        #[ink(message)]
        pub fn decimals(&self) -> u8 {
            18
        }

        #[ink(message)]
        pub fn total_supply(&self) -> u128 {
            self.total_shares
        }

        #[ink(message)]
        pub fn balance_of(&self, owner: AccountId) -> u128 {
            self.shares.get(owner).unwrap_or_default()
        }

        #[ink(message)]
        pub fn allowance(&self, owner: AccountId, spender: AccountId) -> u128 {
            self.allowances.get((owner, spender)).unwrap_or_default()
        }

        #[ink(message)]
        pub fn transfer(&mut self, to: AccountId, value: u128) -> Result<()> {
            let from = self.env().caller();
            self.transfer_from_to(&from, &to, value)
        }

        #[ink(message)]
        pub fn approve(&mut self, spender: AccountId, value: u128) -> Result<()> {
            let owner = self.env().caller();
            self.allowances.insert((&owner, &spender), &value);

            // @bug: https://github.com/paritytech/ink/pull/1243
            // self.env().emit_event(Approval {
            //     owner,
            //     spender,
            //     value,
            // });
            Ok(())
        }

        #[ink(message)]
        pub fn transfer_from(&mut self, from: AccountId, to: AccountId, value: u128) -> Result<()> {
            let caller = self.env().caller();
            let allowance = self.allowance(from, caller);
            if allowance < value {
                return Err(Error::InsufficientAllowance);
            }
            self.transfer_from_to(&from, &to, value)?;
            self.allowances
                .insert((&from, &caller), &(allowance - value));
            Ok(())
        }

        fn transfer_from_to(
            &mut self,
            from: &AccountId,
            to: &AccountId,
            value: u128,
        ) -> Result<()> {
            let from_balance = self.balance_of(*from);
            if from_balance < value {
                return Err(Error::InsufficientBalance);
            }

            self.shares.insert(from, &(from_balance - value));
            let to_balance = self.balance_of(*to);
            self.shares.insert(to, &(to_balance + value));

            // @bug: https://github.com/paritytech/ink/pull/1243
            // self.env().emit_event(Transfer {
            //     from: Some(*from),
            //     to: Some(*to),
            //     value,
            // });

            Ok(())
        }
    }

    #[ink(impl)]
    impl Pool {
        // Returns the liquidity constant of the pool
        fn get_k(&self) -> u128 {
            self.total_token1 * self.total_token2
        }

        // Used to restrict withdraw & swap feature till liquidity is added to the pool
        fn active_pool(&self) -> Result<()> {
            match self.get_k() {
                0 => Err(Error::ZeroLiquidity),
                _ => Ok(()),
            }
        }
    }
}
