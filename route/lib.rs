#![cfg_attr(not(feature = "std"), no_std)]

use ink_lang as ink;

#[ink::contract]
mod route {
    use ink_prelude::vec::Vec;
    use ink_storage::{traits::SpreadAllocate, Mapping};
    use pool::PoolRef;

    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {}

    #[ink(storage)]
    #[derive(SpreadAllocate)]
    pub struct Route {
        pools: Mapping<(AccountId, AccountId), AccountId>,
        fees: u16,
    }

    impl Route {
        #[ink(constructor)]
        pub fn new() -> Self {
            ink_lang::utils::initialize_contract(|contract| Self::new_init(contract))
        }

        fn new_init(&mut self) {}
    }

    impl Route {
        #[ink(message)]
        pub fn create_pool(&mut self, token1: AccountId, token2: AccountId, pool_code_hash: Hash) {
            assert!(
                !self.pair_exists(token1, token2),
                "Given pair already exists"
            );

            let total_balance = Self::env().balance();
            let new_pool: PoolRef = PoolRef::new(token1, token2, self.fees)
                .endowment(total_balance / 4)
                .code_hash(pool_code_hash)
                .salt_bytes(&[0x00])
                .instantiate()
                .unwrap_or_else(|error| {
                    panic!(
                        "failed at instantiating the Accumulator contract: {:?}",
                        error
                    )
                });

            use ink_lang::ToAccountId;
            let addr: AccountId = new_pool.to_account_id();
            self.pools.insert((token1, token2), &addr);
        }

        #[ink(message)]
        pub fn swap_token(&mut self, path: Vec<AccountId>, value: u128) {
            assert!(path.len() >= 2, "Invalid Path");
            let mut current_token = path[0];

            for i in 1..path.len() {
                let next_token = path[i];
                assert!(self.pair_exists(current_token, next_token));
                // @todo: Swap token
                current_token = next_token;
            }
        }
    }

    #[ink(impl)]
    impl Route {
        fn pair_exists(&self, token1: AccountId, token2: AccountId) -> bool {
            if self.pools.get((token1, token2)).is_some() {
                return true;
            }
            self.pools.get((token2, token1)).is_some()
        }
    }
}
