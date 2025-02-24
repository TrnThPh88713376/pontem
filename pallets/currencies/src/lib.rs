// This file is part of Acala.

// Copyright (C) 2020-2021 Acala Foundation.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Currencies module.

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::unused_unit)]
#![allow(clippy::upper_case_acronyms)]

use codec::{Codec, FullCodec};
use frame_support::{
    pallet_prelude::*,
    traits::{
        tokens::{fungibles, fungible, DepositConsequence, WithdrawConsequence},
        Currency as PalletCurrency, ExistenceRequirement, Get, Imbalance,
        LockableCurrency as PalletLockableCurrency,
        ReservableCurrency as PalletReservableCurrency, WithdrawReasons,
    },
    transactional,
};
use frame_system::pallet_prelude::*;
use orml_traits::{
    arithmetic::{Signed, SimpleArithmetic},
    currency::TransferAll,
    BalanceStatus, BasicCurrency, BasicCurrencyExtended, BasicLockableCurrency,
    BasicReservableCurrency, LockIdentifier, MultiCurrency, MultiCurrencyExtended,
    MultiLockableCurrency, MultiReservableCurrency, OnDust,
};
use sp_runtime::{
    traits::{CheckedSub, MaybeSerializeDeserialize, StaticLookup, Zero},
    DispatchError, DispatchResult,
};
use sp_std::{
    convert::{TryFrom, TryInto},
    fmt::Debug,
    marker, result,
    vec::Vec,
};

mod mock;
mod tests;
pub mod weights;

pub use module::*;
pub use weights::WeightInfo;

type BalanceOf<T> = <<T as Config>::MultiCurrency as MultiCurrency<
    <T as frame_system::Config>::AccountId,
>>::Balance;
type CurrencyIdOf<T> = <<T as Config>::MultiCurrency as MultiCurrency<
    <T as frame_system::Config>::AccountId,
>>::CurrencyId;

type AmountOf<T> = <<T as Config>::MultiCurrency as MultiCurrencyExtended<
    <T as frame_system::Config>::AccountId,
>>::Amount;

#[frame_support::pallet]
pub mod module {
    use super::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
        type CurrencyId: FullCodec
            + Eq
            + PartialEq
            + Copy
            + MaybeSerializeDeserialize
            + Debug
            + scale_info::TypeInfo;
        type MultiCurrency: TransferAll<Self::AccountId>
            + MultiCurrencyExtended<Self::AccountId, CurrencyId = Self::CurrencyId>
            + MultiLockableCurrency<Self::AccountId, CurrencyId = Self::CurrencyId>
            + MultiReservableCurrency<Self::AccountId, CurrencyId = Self::CurrencyId>
            + fungibles::Inspect<Self::AccountId>;
        type NativeCurrency: BasicCurrencyExtended<
                Self::AccountId,
                Balance = BalanceOf<Self>,
                Amount = AmountOf<Self>,
            > + BasicLockableCurrency<Self::AccountId, Balance = BalanceOf<Self>>
            + BasicReservableCurrency<Self::AccountId, Balance = BalanceOf<Self>>
            + fungible::Inspect<Self::AccountId>;

        /// The native currency id
        #[pallet::constant]
        type GetNativeCurrencyId: Get<Self::CurrencyId>;

        /// Weight information for extrinsics in this module.
        type WeightInfo: WeightInfo;

        /// The AccountId that can perform a sweep dust.
        type SweepOrigin: EnsureOrigin<Self::Origin>;

        /// Handler to burn or transfer account's dust
        type OnDust: OnDust<Self::AccountId, Self::CurrencyId, BalanceOf<Self>>;
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Unable to convert the Amount type into Balance.
        AmountIntoBalanceFailed,
        /// Balance is too low.
        BalanceTooLow,
        /// Deposit result is not expected
        DepositFailed,
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(crate) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Currency transfer success. \[currency_id, from, to, amount\]
        Transferred(CurrencyIdOf<T>, T::AccountId, T::AccountId, BalanceOf<T>),
        /// Update balance success. \[currency_id, who, amount\]
        BalanceUpdated(CurrencyIdOf<T>, T::AccountId, AmountOf<T>),
        /// Deposit success. \[currency_id, who, amount\]
        Deposited(CurrencyIdOf<T>, T::AccountId, BalanceOf<T>),
        /// Withdraw success. \[currency_id, who, amount\]
        Withdrawn(CurrencyIdOf<T>, T::AccountId, BalanceOf<T>),
        /// Dust swept. \[currency_id, who, amount\]
        DustSwept(CurrencyIdOf<T>, T::AccountId, BalanceOf<T>),
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::hooks]
    impl<T: Config> Hooks<T::BlockNumber> for Pallet<T> {}

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Transfer some balance to another account under `currency_id`.
        ///
        /// The dispatch origin for this call must be `Signed` by the
        /// transactor.
        #[pallet::weight(T::WeightInfo::transfer_non_native_currency())]
        pub fn transfer(
            origin: OriginFor<T>,
            dest: <T::Lookup as StaticLookup>::Source,
            currency_id: CurrencyIdOf<T>,
            #[pallet::compact] amount: BalanceOf<T>,
        ) -> DispatchResult {
            let from = ensure_signed(origin)?;
            let to = T::Lookup::lookup(dest)?;
            <Self as MultiCurrency<T::AccountId>>::transfer(currency_id, &from, &to, amount)?;
            Ok(())
        }

        /// Transfer some native currency to another account.
        ///
        /// The dispatch origin for this call must be `Signed` by the
        /// transactor.
        #[pallet::weight(T::WeightInfo::transfer_native_currency())]
        pub fn transfer_native_currency(
            origin: OriginFor<T>,
            dest: <T::Lookup as StaticLookup>::Source,
            #[pallet::compact] amount: BalanceOf<T>,
        ) -> DispatchResult {
            let from = ensure_signed(origin)?;
            let to = T::Lookup::lookup(dest)?;
            T::NativeCurrency::transfer(&from, &to, amount)?;

            Self::deposit_event(Event::Transferred(
                T::GetNativeCurrencyId::get(),
                from,
                to,
                amount,
            ));
            Ok(())
        }

        /// update amount of account `who` under `currency_id`.
        ///
        /// The dispatch origin of this call must be _Root_.
        #[pallet::weight(T::WeightInfo::update_balance_non_native_currency())]
        pub fn update_balance(
            origin: OriginFor<T>,
            who: <T::Lookup as StaticLookup>::Source,
            currency_id: CurrencyIdOf<T>,
            amount: AmountOf<T>,
        ) -> DispatchResult {
            ensure_root(origin)?;
            let dest = T::Lookup::lookup(who)?;
            <Self as MultiCurrencyExtended<T::AccountId>>::update_balance(
                currency_id,
                &dest,
                amount,
            )?;
            Ok(())
        }

        #[pallet::weight(T::WeightInfo::sweep_dust(accounts.len() as u32))]
        pub fn sweep_dust(
            origin: OriginFor<T>,
            currency_id: CurrencyIdOf<T>,
            accounts: Vec<T::AccountId>,
        ) -> DispatchResult {
            T::SweepOrigin::ensure_origin(origin)?;

            for account in accounts {
                let free_balance = Self::free_balance(currency_id, &account);
                if free_balance.is_zero() {
                    continue;
                }
                let total_balance = Self::total_balance(currency_id, &account);
                if free_balance != total_balance {
                    continue;
                }
                if free_balance < Self::minimum_balance(currency_id) {
                    T::OnDust::on_dust(&account, currency_id, free_balance);
                    Self::deposit_event(Event::DustSwept(currency_id, account, free_balance));
                }
            }
            Ok(())
        }
    }
}

impl<T: Config> MultiCurrency<T::AccountId> for Pallet<T> {
    type CurrencyId = CurrencyIdOf<T>;
    type Balance = BalanceOf<T>;

    fn minimum_balance(currency_id: Self::CurrencyId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::minimum_balance(),
            _ => T::MultiCurrency::minimum_balance(currency_id),
        }
    }

    fn total_issuance(currency_id: Self::CurrencyId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::total_issuance(),
            _ => T::MultiCurrency::total_issuance(currency_id),
        }
    }

    fn total_balance(currency_id: Self::CurrencyId, who: &T::AccountId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::total_balance(who),
            _ => T::MultiCurrency::total_balance(currency_id, who),
        }
    }

    fn free_balance(currency_id: Self::CurrencyId, who: &T::AccountId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::free_balance(who),
            _ => T::MultiCurrency::free_balance(currency_id, who),
        }
    }

    fn ensure_can_withdraw(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::ensure_can_withdraw(who, amount)
            }
            _ => T::MultiCurrency::ensure_can_withdraw(currency_id, who, amount),
        }
    }

    fn transfer(
        currency_id: Self::CurrencyId,
        from: &T::AccountId,
        to: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        if amount.is_zero() || from == to {
            return Ok(());
        }

        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::transfer(from, to, amount)?
            }
            _ => T::MultiCurrency::transfer(currency_id, from, to, amount)?,
        }

        Self::deposit_event(Event::Transferred(
            currency_id,
            from.clone(),
            to.clone(),
            amount,
        ));
        Ok(())
    }

    fn deposit(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        if amount.is_zero() {
            return Ok(());
        }
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::deposit(who, amount)?,
            _ => T::MultiCurrency::deposit(currency_id, who, amount)?,
        }
        Self::deposit_event(Event::Deposited(currency_id, who.clone(), amount));
        Ok(())
    }

    fn withdraw(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        if amount.is_zero() {
            return Ok(());
        }
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::withdraw(who, amount)?
            }
            _ => T::MultiCurrency::withdraw(currency_id, who, amount)?,
        }
        Self::deposit_event(Event::Withdrawn(currency_id, who.clone(), amount));
        Ok(())
    }

    fn can_slash(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> bool {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::can_slash(who, amount)
            }
            _ => T::MultiCurrency::can_slash(currency_id, who, amount),
        }
    }

    fn slash(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::slash(who, amount),
            _ => T::MultiCurrency::slash(currency_id, who, amount),
        }
    }
}

impl<T: Config> MultiCurrencyExtended<T::AccountId> for Pallet<T> {
    type Amount = AmountOf<T>;

    fn update_balance(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        by_amount: Self::Amount,
    ) -> DispatchResult {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::update_balance(who, by_amount)?
            }
            _ => T::MultiCurrency::update_balance(currency_id, who, by_amount)?,
        }
        Self::deposit_event(Event::BalanceUpdated(currency_id, who.clone(), by_amount));
        Ok(())
    }
}

impl<T: Config> MultiLockableCurrency<T::AccountId> for Pallet<T> {
    type Moment = T::BlockNumber;

    fn set_lock(
        lock_id: LockIdentifier,
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::set_lock(lock_id, who, amount)
            }
            _ => T::MultiCurrency::set_lock(lock_id, currency_id, who, amount),
        }
    }

    fn extend_lock(
        lock_id: LockIdentifier,
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::extend_lock(lock_id, who, amount)
            }
            _ => T::MultiCurrency::extend_lock(lock_id, currency_id, who, amount),
        }
    }

    fn remove_lock(
        lock_id: LockIdentifier,
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
    ) -> DispatchResult {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::remove_lock(lock_id, who)
            }
            _ => T::MultiCurrency::remove_lock(lock_id, currency_id, who),
        }
    }
}

impl<T: Config> MultiReservableCurrency<T::AccountId> for Pallet<T> {
    fn can_reserve(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        value: Self::Balance,
    ) -> bool {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::can_reserve(who, value)
            }
            _ => T::MultiCurrency::can_reserve(currency_id, who, value),
        }
    }

    fn slash_reserved(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        value: Self::Balance,
    ) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::slash_reserved(who, value)
            }
            _ => T::MultiCurrency::slash_reserved(currency_id, who, value),
        }
    }

    fn reserved_balance(currency_id: Self::CurrencyId, who: &T::AccountId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::reserved_balance(who),
            _ => T::MultiCurrency::reserved_balance(currency_id, who),
        }
    }

    fn reserve(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        value: Self::Balance,
    ) -> DispatchResult {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::reserve(who, value),
            _ => T::MultiCurrency::reserve(currency_id, who, value),
        }
    }

    fn unreserve(
        currency_id: Self::CurrencyId,
        who: &T::AccountId,
        value: Self::Balance,
    ) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => T::NativeCurrency::unreserve(who, value),
            _ => T::MultiCurrency::unreserve(currency_id, who, value),
        }
    }

    fn repatriate_reserved(
        currency_id: Self::CurrencyId,
        slashed: &T::AccountId,
        beneficiary: &T::AccountId,
        value: Self::Balance,
        status: BalanceStatus,
    ) -> result::Result<Self::Balance, DispatchError> {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::repatriate_reserved(slashed, beneficiary, value, status)
            }
            _ => T::MultiCurrency::repatriate_reserved(
                currency_id,
                slashed,
                beneficiary,
                value,
                status,
            ),
        }
    }
}

/// Implement 'fungibles:Inspect' for Pallet.
impl<T: Config> fungibles::Inspect<T::AccountId> for Pallet<T>
where
    T::NativeCurrency: fungible::Inspect<T::AccountId, Balance = BalanceOf<T>>,
    T::MultiCurrency:
        fungibles::Inspect<T::AccountId, Balance = BalanceOf<T>, AssetId = T::CurrencyId>,
{
    type Balance = BalanceOf<T>;
    type AssetId = T::CurrencyId;

    fn total_issuance(currency_id: Self::AssetId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                <T::NativeCurrency as fungible::Inspect<T::AccountId>>::total_issuance()
            }
            _ => <T::MultiCurrency as fungibles::Inspect<T::AccountId>>::total_issuance(
                currency_id,
            ),
        }
    }

    fn minimum_balance(currency_id: Self::AssetId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                <T::NativeCurrency as fungible::Inspect<T::AccountId>>::minimum_balance()
            }
            _ => <T::MultiCurrency as fungibles::Inspect<T::AccountId>>::minimum_balance(
                currency_id,
            ),
        }
    }

    fn balance(currency_id: Self::AssetId, who: &T::AccountId) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                <T::NativeCurrency as fungible::Inspect<T::AccountId>>::balance(who)
            }
            _ => {
                <T::MultiCurrency as fungibles::Inspect<T::AccountId>>::balance(currency_id, who)
            }
        }
    }

    fn reducible_balance(
        currency_id: Self::AssetId,
        who: &T::AccountId,
        keep_alive: bool,
    ) -> Self::Balance {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                <T::NativeCurrency as fungible::Inspect<T::AccountId>>::reducible_balance(
                    who, keep_alive,
                )
            }
            _ => <T::MultiCurrency as fungibles::Inspect<T::AccountId>>::reducible_balance(
                currency_id,
                who,
                keep_alive,
            ),
        }
    }

    fn can_deposit(
        currency_id: Self::AssetId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DepositConsequence {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                <T::NativeCurrency as fungible::Inspect<T::AccountId>>::can_deposit(who, amount)
            }
            _ => <T::MultiCurrency as fungibles::Inspect<T::AccountId>>::can_deposit(
                currency_id,
                who,
                amount,
            ),
        }
    }

    fn can_withdraw(
        currency_id: Self::AssetId,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> WithdrawConsequence<Self::Balance> {
        match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                <T::NativeCurrency as fungible::Inspect<T::AccountId>>::can_withdraw(who, amount)
            }
            _ => <T::MultiCurrency as fungibles::Inspect<T::AccountId>>::can_withdraw(
                currency_id,
                who,
                amount,
            ),
        }
    }
}

pub struct Currency<T, GetCurrencyId>(marker::PhantomData<T>, marker::PhantomData<GetCurrencyId>);

impl<T, GetCurrencyId> BasicCurrency<T::AccountId> for Currency<T, GetCurrencyId>
where
    T: Config,
    GetCurrencyId: Get<CurrencyIdOf<T>>,
{
    type Balance = BalanceOf<T>;

    fn minimum_balance() -> Self::Balance {
        <Pallet<T>>::minimum_balance(GetCurrencyId::get())
    }

    fn total_issuance() -> Self::Balance {
        <Pallet<T>>::total_issuance(GetCurrencyId::get())
    }

    fn total_balance(who: &T::AccountId) -> Self::Balance {
        <Pallet<T>>::total_balance(GetCurrencyId::get(), who)
    }

    fn free_balance(who: &T::AccountId) -> Self::Balance {
        <Pallet<T>>::free_balance(GetCurrencyId::get(), who)
    }

    fn ensure_can_withdraw(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
        <Pallet<T>>::ensure_can_withdraw(GetCurrencyId::get(), who, amount)
    }

    fn transfer(from: &T::AccountId, to: &T::AccountId, amount: Self::Balance) -> DispatchResult {
        <Pallet<T> as MultiCurrency<T::AccountId>>::transfer(
            GetCurrencyId::get(),
            from,
            to,
            amount,
        )
    }

    fn deposit(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
        <Pallet<T>>::deposit(GetCurrencyId::get(), who, amount)
    }

    fn withdraw(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
        <Pallet<T>>::withdraw(GetCurrencyId::get(), who, amount)
    }

    fn can_slash(who: &T::AccountId, amount: Self::Balance) -> bool {
        <Pallet<T>>::can_slash(GetCurrencyId::get(), who, amount)
    }

    fn slash(who: &T::AccountId, amount: Self::Balance) -> Self::Balance {
        <Pallet<T>>::slash(GetCurrencyId::get(), who, amount)
    }
}

impl<T, GetCurrencyId> BasicCurrencyExtended<T::AccountId> for Currency<T, GetCurrencyId>
where
    T: Config,
    GetCurrencyId: Get<CurrencyIdOf<T>>,
{
    type Amount = AmountOf<T>;

    fn update_balance(who: &T::AccountId, by_amount: Self::Amount) -> DispatchResult {
        <Pallet<T> as MultiCurrencyExtended<T::AccountId>>::update_balance(
            GetCurrencyId::get(),
            who,
            by_amount,
        )
    }
}

impl<T, GetCurrencyId> BasicLockableCurrency<T::AccountId> for Currency<T, GetCurrencyId>
where
    T: Config,
    GetCurrencyId: Get<CurrencyIdOf<T>>,
{
    type Moment = T::BlockNumber;

    fn set_lock(
        lock_id: LockIdentifier,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        <Pallet<T> as MultiLockableCurrency<T::AccountId>>::set_lock(
            lock_id,
            GetCurrencyId::get(),
            who,
            amount,
        )
    }

    fn extend_lock(
        lock_id: LockIdentifier,
        who: &T::AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        <Pallet<T> as MultiLockableCurrency<T::AccountId>>::extend_lock(
            lock_id,
            GetCurrencyId::get(),
            who,
            amount,
        )
    }

    fn remove_lock(lock_id: LockIdentifier, who: &T::AccountId) -> DispatchResult {
        <Pallet<T> as MultiLockableCurrency<T::AccountId>>::remove_lock(
            lock_id,
            GetCurrencyId::get(),
            who,
        )
    }
}

impl<T, GetCurrencyId> BasicReservableCurrency<T::AccountId> for Currency<T, GetCurrencyId>
where
    T: Config,
    GetCurrencyId: Get<CurrencyIdOf<T>>,
{
    fn can_reserve(who: &T::AccountId, value: Self::Balance) -> bool {
        <Pallet<T> as MultiReservableCurrency<T::AccountId>>::can_reserve(
            GetCurrencyId::get(),
            who,
            value,
        )
    }

    fn slash_reserved(who: &T::AccountId, value: Self::Balance) -> Self::Balance {
        <Pallet<T> as MultiReservableCurrency<T::AccountId>>::slash_reserved(
            GetCurrencyId::get(),
            who,
            value,
        )
    }

    fn reserved_balance(who: &T::AccountId) -> Self::Balance {
        <Pallet<T> as MultiReservableCurrency<T::AccountId>>::reserved_balance(
            GetCurrencyId::get(),
            who,
        )
    }

    fn reserve(who: &T::AccountId, value: Self::Balance) -> DispatchResult {
        <Pallet<T> as MultiReservableCurrency<T::AccountId>>::reserve(
            GetCurrencyId::get(),
            who,
            value,
        )
    }

    fn unreserve(who: &T::AccountId, value: Self::Balance) -> Self::Balance {
        <Pallet<T> as MultiReservableCurrency<T::AccountId>>::unreserve(
            GetCurrencyId::get(),
            who,
            value,
        )
    }

    fn repatriate_reserved(
        slashed: &T::AccountId,
        beneficiary: &T::AccountId,
        value: Self::Balance,
        status: BalanceStatus,
    ) -> result::Result<Self::Balance, DispatchError> {
        <Pallet<T> as MultiReservableCurrency<T::AccountId>>::repatriate_reserved(
            GetCurrencyId::get(),
            slashed,
            beneficiary,
            value,
            status,
        )
    }
}

/// Adapt other currency traits implementation to `BasicCurrency`.
pub struct BasicCurrencyAdapter<T, Currency, Amount, Moment>(
    marker::PhantomData<(T, Currency, Amount, Moment)>,
);

type PalletBalanceOf<A, Currency> = <Currency as PalletCurrency<A>>::Balance;

// Adapt `frame_support::traits::Currency`
impl<T, AccountId, Currency, Amount, Moment> BasicCurrency<AccountId>
    for BasicCurrencyAdapter<T, Currency, Amount, Moment>
where
    Currency: PalletCurrency<AccountId>,
    T: Config,
{
    type Balance = PalletBalanceOf<AccountId, Currency>;

    fn minimum_balance() -> Self::Balance {
        Currency::minimum_balance()
    }

    fn total_issuance() -> Self::Balance {
        Currency::total_issuance()
    }

    fn total_balance(who: &AccountId) -> Self::Balance {
        Currency::total_balance(who)
    }

    fn free_balance(who: &AccountId) -> Self::Balance {
        Currency::free_balance(who)
    }

    fn ensure_can_withdraw(who: &AccountId, amount: Self::Balance) -> DispatchResult {
        let new_balance = Self::free_balance(who)
            .checked_sub(&amount)
            .ok_or(Error::<T>::BalanceTooLow)?;

        Currency::ensure_can_withdraw(who, amount, WithdrawReasons::all(), new_balance)
    }

    fn transfer(from: &AccountId, to: &AccountId, amount: Self::Balance) -> DispatchResult {
        Currency::transfer(from, to, amount, ExistenceRequirement::AllowDeath)
    }

    fn deposit(who: &AccountId, amount: Self::Balance) -> DispatchResult {
        if !amount.is_zero() {
            let deposit_result = Currency::deposit_creating(who, amount);
            let actual_deposit = deposit_result.peek();
            ensure!(actual_deposit == amount, Error::<T>::DepositFailed);
        }

        Ok(())
    }

    fn withdraw(who: &AccountId, amount: Self::Balance) -> DispatchResult {
        Currency::withdraw(
            who,
            amount,
            WithdrawReasons::all(),
            ExistenceRequirement::AllowDeath,
        )
        .map(|_| ())
    }

    fn can_slash(who: &AccountId, amount: Self::Balance) -> bool {
        Currency::can_slash(who, amount)
    }

    fn slash(who: &AccountId, amount: Self::Balance) -> Self::Balance {
        let (_, gap) = Currency::slash(who, amount);
        gap
    }
}

// Adapt `frame_support::traits::Currency`
impl<T, AccountId, Currency, Amount, Moment> BasicCurrencyExtended<AccountId>
    for BasicCurrencyAdapter<T, Currency, Amount, Moment>
where
    Amount: Signed
        + TryInto<PalletBalanceOf<AccountId, Currency>>
        + TryFrom<PalletBalanceOf<AccountId, Currency>>
        + SimpleArithmetic
        + Codec
        + Copy
        + MaybeSerializeDeserialize
        + Debug
        + Default,
    Currency: PalletCurrency<AccountId>,
    T: Config,
{
    type Amount = Amount;

    fn update_balance(who: &AccountId, by_amount: Self::Amount) -> DispatchResult {
        let by_balance = by_amount
            .abs()
            .try_into()
            .map_err(|_| Error::<T>::AmountIntoBalanceFailed)?;
        if by_amount.is_positive() {
            Self::deposit(who, by_balance)
        } else {
            Self::withdraw(who, by_balance)
        }
    }
}

// Adapt `frame_support::traits::LockableCurrency`
impl<T, AccountId, Currency, Amount, Moment> BasicLockableCurrency<AccountId>
    for BasicCurrencyAdapter<T, Currency, Amount, Moment>
where
    Currency: PalletLockableCurrency<AccountId>,
    T: Config,
{
    type Moment = Moment;

    fn set_lock(
        lock_id: LockIdentifier,
        who: &AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        Currency::set_lock(lock_id, who, amount, WithdrawReasons::all());
        Ok(())
    }

    fn extend_lock(
        lock_id: LockIdentifier,
        who: &AccountId,
        amount: Self::Balance,
    ) -> DispatchResult {
        Currency::extend_lock(lock_id, who, amount, WithdrawReasons::all());
        Ok(())
    }

    fn remove_lock(lock_id: LockIdentifier, who: &AccountId) -> DispatchResult {
        Currency::remove_lock(lock_id, who);
        Ok(())
    }
}

// Adapt `frame_support::traits::ReservableCurrency`
impl<T, AccountId, Currency, Amount, Moment> BasicReservableCurrency<AccountId>
    for BasicCurrencyAdapter<T, Currency, Amount, Moment>
where
    Currency: PalletReservableCurrency<AccountId>,
    T: Config,
{
    fn can_reserve(who: &AccountId, value: Self::Balance) -> bool {
        Currency::can_reserve(who, value)
    }

    fn slash_reserved(who: &AccountId, value: Self::Balance) -> Self::Balance {
        let (_, gap) = Currency::slash_reserved(who, value);
        gap
    }

    fn reserved_balance(who: &AccountId) -> Self::Balance {
        Currency::reserved_balance(who)
    }

    fn reserve(who: &AccountId, value: Self::Balance) -> DispatchResult {
        Currency::reserve(who, value)
    }

    fn unreserve(who: &AccountId, value: Self::Balance) -> Self::Balance {
        Currency::unreserve(who, value)
    }

    fn repatriate_reserved(
        slashed: &AccountId,
        beneficiary: &AccountId,
        value: Self::Balance,
        status: BalanceStatus,
    ) -> result::Result<Self::Balance, DispatchError> {
        Currency::repatriate_reserved(slashed, beneficiary, value, status)
    }
}

// Adapt 'fungible::Inspect'
impl<T, AccountId, Currency, Amount, Moment> fungible::Inspect<AccountId>
    for BasicCurrencyAdapter<T, Currency, Amount, Moment>
where
    T: Config,
    Currency: fungible::Inspect<AccountId, Balance = BalanceOf<T>>,
{
    type Balance = BalanceOf<T>;

    fn total_issuance() -> Self::Balance {
        Currency::total_issuance()
    }

    fn minimum_balance() -> Self::Balance {
        Currency::minimum_balance()
    }

    fn balance(who: &AccountId) -> Self::Balance {
        Currency::balance(who)
    }

    fn reducible_balance(who: &AccountId, keep_alive: bool) -> Self::Balance {
        Currency::reducible_balance(who, keep_alive)
    }

    fn can_deposit(who: &AccountId, amount: Self::Balance) -> DepositConsequence {
        Currency::can_deposit(who, amount)
    }

    fn can_withdraw(
        who: &AccountId,
        amount: Self::Balance,
    ) -> WithdrawConsequence<Self::Balance> {
        Currency::can_withdraw(who, amount)
    }
}

impl<T: Config> TransferAll<T::AccountId> for Pallet<T> {
    #[transactional]
    fn transfer_all(source: &T::AccountId, dest: &T::AccountId) -> DispatchResult {
        // transfer non-native free to dest
        T::MultiCurrency::transfer_all(source, dest)?;

        // transfer all free to dest
        T::NativeCurrency::transfer(source, dest, T::NativeCurrency::free_balance(source))
    }
}

pub struct TransferDust<T, GetAccountId>(marker::PhantomData<(T, GetAccountId)>);
impl<T: Config, GetAccountId> OnDust<T::AccountId, CurrencyIdOf<T>, BalanceOf<T>>
    for TransferDust<T, GetAccountId>
where
    T: Config,
    GetAccountId: Get<T::AccountId>,
{
    fn on_dust(who: &T::AccountId, currency_id: CurrencyIdOf<T>, amount: BalanceOf<T>) {
        // transfer the dust to treasury account, ignore the result,
        // if failed will leave some dust which still could be recycled.
        let _ = match currency_id {
            id if id == T::GetNativeCurrencyId::get() => {
                T::NativeCurrency::transfer(who, &GetAccountId::get(), amount)
            }
            _ => T::MultiCurrency::transfer(currency_id, who, &GetAccountId::get(), amount),
        };
    }
}
