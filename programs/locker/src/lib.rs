use std::ops::DerefMut;

use anchor_lang::{
    prelude::*,
    solana_program::{
        self,
        log::{sol_log, sol_log_64},
    },
    AccountsClose,
};
use anchor_spl::{
    associated_token::get_associated_token_address,
    token::{self, CloseAccount, Mint, Token, TokenAccount, Transfer},
};

use az::CheckedAs;

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[error]
pub enum ErrorCode {
    #[msg("The given unlock date is in the past")]
    UnlockInThePast,
    InvalidTimestamp,
    #[msg("The given fee wallet is not associated with required fee wallet")]
    InvalidFeeWallet,
    IntegerOverflow,
    NothingToLock,
    InvalidAmountTransferred,
    InvalidPeriod,
    CannotUnlockToEarlierDate,
    TooEarlyToWithdraw,
    InvalidAmount,
    InvalidCountry,
    InitMintInfoNotAuthorized,
    LinearEmissionDisabled,
}

#[program]
pub mod locker {
    use super::*;

    pub fn init_config(ctx: Context<InitConfig>, args: CreateConfigArgs) -> Result<()> {
        let config = ctx.accounts.config.deref_mut();

        *config = Config {
            admin: ctx.accounts.admin.key(),
            fee_in_sol: args.fee_in_sol,
            fee_in_token_numerator: args.fee_in_token_numerator,
            fee_in_token_denominator: args.fee_in_token_denominator,
            mint_info_permissioned: args.mint_info_permissioned,
            has_linear_emission: args.has_linear_emission,
            fee_wallet: ctx.accounts.fee_wallet.key(),
            country_list: ctx.accounts.country_list.key(),
            bump: args.bump,
        };

        Ok(())
    }

    pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
        let config = &mut ctx.accounts.config;
        let UpdateConfigArgs {
            fee_in_sol,
            fee_in_token_numerator,
            fee_in_token_denominator,
            mint_info_permissioned,
            has_linear_emission,
        } = args;

        config.fee_in_sol = fee_in_sol.unwrap_or(config.fee_in_sol);
        config.fee_in_token_numerator =
            fee_in_token_numerator.unwrap_or(config.fee_in_token_numerator);
        config.fee_in_token_denominator =
            fee_in_token_denominator.unwrap_or(config.fee_in_token_denominator);
        config.mint_info_permissioned =
            mint_info_permissioned.unwrap_or(config.mint_info_permissioned);
        config.has_linear_emission = has_linear_emission.unwrap_or(config.has_linear_emission);

        config.fee_wallet = ctx.accounts.fee_wallet.key();
        config.country_list = ctx.accounts.country_list.key();

        Ok(())
    }

    pub fn init_mint_info(ctx: Context<InitMintInfo>, bump: u8) -> Result<()> {
        let mint_info = ctx.accounts.mint_info.deref_mut();

        if ctx.accounts.config.mint_info_permissioned {
            require!(
                ctx.accounts.payer.key() == ctx.accounts.config.admin,
                InitMintInfoNotAuthorized
            );
        }

        *mint_info = MintInfo {
            bump,
            fee_paid: false,
        };

        sol_log("Initialize mint info");

        Ok(())
    }

    pub fn create_locker<'info>(
        ctx: Context<'_, '_, '_, 'info, CreateLocker<'info>>,
        args: CreateLockerArgs,
    ) -> Result<()> {
        sol_log("Create locker: start");

        let now = ctx.accounts.clock.unix_timestamp;
        require!(args.unlock_date > now, UnlockInThePast);
        // prevents errors when timestamp entered as milliseconds
        require!(args.unlock_date < 10000000000, InvalidTimestamp);

        let config = &ctx.accounts.config;

        if !config.has_linear_emission {
            require!(args.start_emission.is_none(), LinearEmissionDisabled);
        }

        if let Some(start_emission) = args.start_emission {
            require!(args.unlock_date > start_emission, InvalidPeriod);
        }

        require!(
            ctx.accounts
                .country_banlist
                .is_country_valid(&args.country_code),
            InvalidCountry
        );

        let mint_info = &mut ctx.accounts.mint_info;

        if should_pay_in_sol(config, mint_info, args.fee_in_sol) {
            FeeInSol {
                fee_wallet: &ctx.accounts.fee_wallet,
                payer: &ctx.accounts.owner,
                config,
                mint_info,
                system_program: &ctx.accounts.system_program,
            }
            .pay()?;
        }

        let lock_fee = if should_pay_in_tokens(config, mint_info, args.fee_in_sol) {
            FeeInTokens {
                config,
                funding_wallet: &mut ctx.accounts.funding_wallet,
                funding_wallet_authority: &ctx.accounts.funding_wallet_authority,
                fee_wallet: &ctx.accounts.fee_token_wallet,
                amount: args.amount,
                token_program: &ctx.accounts.token_program,
            }
            .pay()?
        } else {
            0
        };

        let amount_to_lock = args.amount - lock_fee;
        require!(amount_to_lock > 0, NothingToLock);

        let locker = ctx.accounts.locker.deref_mut();

        *locker = Locker {
            owner: ctx.accounts.owner.key(),
            country_code: country_list::string_to_byte_array(&args.country_code),
            current_unlock_date: args.unlock_date,
            start_emission: args.start_emission,
            deposited_amount: amount_to_lock,
            vault: ctx.accounts.vault.key(),
            vault_bump: args.vault_bump,
        };

        TokenTransfer {
            amount: amount_to_lock,
            from: &mut ctx.accounts.funding_wallet,
            to: &ctx.accounts.vault,
            authority: &ctx.accounts.funding_wallet_authority,
            token_program: &ctx.accounts.token_program,
            signers: None,
        }
        .make()?;

        Ok(())
    }

    pub fn relock(ctx: Context<Relock>, unlock_date: i64) -> Result<()> {
        let locker = &mut ctx.accounts.locker;

        require!(
            unlock_date > locker.current_unlock_date,
            CannotUnlockToEarlierDate
        );

        locker.current_unlock_date = unlock_date;

        Ok(())
    }

    pub fn transfer_ownership(ctx: Context<TransferOwnership>) -> Result<()> {
        let locker = &mut ctx.accounts.locker;

        locker.owner = ctx.accounts.new_owner.key();

        Ok(())
    }

    pub fn increment_lock(ctx: Context<IncrementLock>, amount: u64) -> Result<()> {
        let locker = &mut ctx.accounts.locker;
        let mint_info = &ctx.accounts.mint_info;
        let config = &ctx.accounts.config;

        // 3rd argument is false b/c we do not pay in sol here at all
        let amount_to_lock = if should_pay_in_tokens(config, mint_info, false) {
            let lock_fee = mul_div(
                amount,
                config.fee_in_token_numerator,
                config.fee_in_token_denominator,
            )
            .ok_or(ErrorCode::IntegerOverflow)?;

            FeeInTokens {
                config,
                funding_wallet: &mut ctx.accounts.funding_wallet,
                funding_wallet_authority: &ctx.accounts.funding_wallet_authority,
                fee_wallet: &ctx.accounts.fee_wallet,
                amount: lock_fee,
                token_program: &ctx.accounts.token_program,
            }
            .pay()?;

            amount - lock_fee
        } else {
            amount
        };

        TokenTransfer {
            amount: amount_to_lock,
            from: &mut ctx.accounts.funding_wallet,
            to: &ctx.accounts.vault,
            authority: &ctx.accounts.funding_wallet_authority,
            token_program: &ctx.accounts.token_program,
            signers: None,
        }
        .make()?;

        locker.deposited_amount = locker
            .deposited_amount
            .checked_add(amount_to_lock)
            .ok_or(ErrorCode::IntegerOverflow)?;

        Ok(())
    }

    pub fn withdraw_funds(ctx: Context<WithdrawFunds>, amount: u64) -> Result<()> {
        let now = ctx.accounts.clock.unix_timestamp;
        let locker = &ctx.accounts.locker;
        let vault = &mut ctx.accounts.vault;

        let amount_to_transfer = match locker.start_emission {
            Some(start_emission) => {
                let clamped_time = now.clamp(start_emission, locker.current_unlock_date);
                let elapsed = clamped_time - start_emission;
                let full_period = locker.current_unlock_date - start_emission;
                require!(full_period > 0, InvalidPeriod);

                sol_log_64(
                    amount,
                    elapsed as u64,
                    full_period as u64,
                    now as u64,
                    start_emission as u64,
                );

                mul_div(locker.deposited_amount, elapsed, full_period as u64)
                    .ok_or(ErrorCode::IntegerOverflow)?
                    .min(amount)
            }
            None => {
                require!(now > locker.current_unlock_date, TooEarlyToWithdraw);
                amount.min(vault.amount)
            }
        };

        require!(amount_to_transfer > 0, InvalidAmount);
        require!(amount_to_transfer <= vault.amount, InvalidAmount);

        let locker_key = locker.key();
        let seeds = &[locker_key.as_ref(), &[locker.vault_bump]];
        let signers = &[&seeds[..]];

        TokenTransfer {
            amount: amount_to_transfer,
            from: vault,
            to: &ctx.accounts.target_wallet,
            authority: &ctx.accounts.vault_authority,
            token_program: &ctx.accounts.token_program,
            signers: Some(signers),
        }
        .make()?;

        vault.reload()?;
        if vault.amount == 0 {
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                CloseAccount {
                    account: vault.to_account_info(),
                    destination: ctx.accounts.owner.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signers,
            );
            token::close_account(cpi_ctx)?;

            locker.close(ctx.accounts.owner.to_account_info())?;
        }

        Ok(())
    }

    pub fn split_locker(ctx: Context<SplitLocker>, args: SplitLockerArgs) -> Result<()> {
        require!(args.amount > 0, InvalidAmount);

        let new_locker = ctx.accounts.new_locker.deref_mut();
        let old_locker = &mut ctx.accounts.old_locker;
        let old_vault = &mut ctx.accounts.old_vault;

        require!(args.amount <= old_vault.amount, InvalidAmount);

        let locker_key = old_locker.key();
        let seeds = &[locker_key.as_ref(), &[old_locker.vault_bump]];
        let signers = &[&seeds[..]];

        TokenTransfer {
            amount: args.amount,
            from: old_vault,
            to: &ctx.accounts.new_vault,
            authority: &ctx.accounts.old_vault_authority,
            token_program: &ctx.accounts.token_program,
            signers: Some(signers),
        }
        .make()?;

        old_locker.deposited_amount = old_locker
            .deposited_amount
            .checked_sub(args.amount)
            .ok_or(ErrorCode::IntegerOverflow)?;

        old_vault.reload()?;
        if old_vault.amount == 0 {
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                CloseAccount {
                    account: old_vault.to_account_info(),
                    destination: ctx.accounts.old_owner.to_account_info(),
                    authority: ctx.accounts.old_vault_authority.to_account_info(),
                },
                signers,
            );
            token::close_account(cpi_ctx)?;

            old_locker.close(ctx.accounts.old_owner.to_account_info())?;
        }

        *new_locker = Locker {
            owner: ctx.accounts.new_owner.key(),
            country_code: old_locker.country_code,
            current_unlock_date: old_locker.current_unlock_date,
            start_emission: old_locker.start_emission,
            deposited_amount: args.amount,
            vault: ctx.accounts.new_vault.key(),
            vault_bump: args.vault_bump,
        };

        Ok(())
    }

    pub fn close_locker(ctx: Context<CloseLocker>) -> Result<()> {
        let locker = &ctx.accounts.locker;
        let vault = &mut ctx.accounts.vault;

        let locker_key = locker.key();
        let seeds = &[locker_key.as_ref(), &[locker.vault_bump]];
        let signers = &[&seeds[..]];

        TokenTransfer {
            amount: vault.amount,
            from: vault,
            to: &ctx.accounts.target_wallet,
            authority: &ctx.accounts.vault_authority,
            token_program: &ctx.accounts.token_program,
            signers: Some(signers),
        }
        .make()?;

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: vault.to_account_info(),
                destination: ctx.accounts.owner.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signers,
        );
        token::close_account(cpi_ctx)?;

        locker.close(ctx.accounts.owner.to_account_info())?;

        Ok(())
    }
}

#[account]
#[derive(Debug)]
pub struct Config {
    admin: Pubkey,
    fee_in_sol: u64,
    fee_in_token_numerator: u64,
    fee_in_token_denominator: u64,
    mint_info_permissioned: bool,
    has_linear_emission: bool,
    fee_wallet: Pubkey,
    country_list: Pubkey,
    bump: u8,
}

impl Config {
    pub const LEN: usize = 8 + std::mem::size_of::<Self>();
}

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct CreateConfigArgs {
    pub fee_in_sol: u64,
    pub fee_in_token_numerator: u64,
    pub fee_in_token_denominator: u64,
    pub mint_info_permissioned: bool,
    pub has_linear_emission: bool,
    pub bump: u8,
}

#[derive(Accounts)]
#[instruction(args: CreateConfigArgs)]
pub struct InitConfig<'info> {
    #[account(signer)]
    admin: AccountInfo<'info>,
    #[account(
        init,
        payer = admin,
        seeds = [
            "config".as_ref()
        ],
        bump = args.bump,
        space = Config::LEN
    )]
    config: ProgramAccount<'info, Config>,
    fee_wallet: AccountInfo<'info>,
    country_list: Account<'info, country_list::CountryBanList>,

    system_program: Program<'info, System>,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct UpdateConfigArgs {
    fee_in_sol: Option<u64>,
    fee_in_token_numerator: Option<u64>,
    fee_in_token_denominator: Option<u64>,
    mint_info_permissioned: Option<bool>,
    has_linear_emission: Option<bool>,
}

#[derive(Accounts)]
#[instruction(args: UpdateConfigArgs)]
pub struct UpdateConfig<'info> {
    #[account(signer)]
    admin: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [
            "config".as_ref()
        ],
        bump = config.bump,
        constraint = config.admin == admin.key()
    )]
    config: ProgramAccount<'info, Config>,

    fee_wallet: AccountInfo<'info>,
    country_list: Account<'info, country_list::CountryBanList>,
}

#[account]
pub struct Locker {
    owner: Pubkey,
    country_code: [u8; 2],
    current_unlock_date: i64,
    start_emission: Option<i64>,
    deposited_amount: u64,
    vault: Pubkey,
    vault_bump: u8,
}

impl Locker {
    pub const LEN: usize = std::mem::size_of::<Self>() + 8;
}

#[account]
pub struct MintInfo {
    bump: u8,
    fee_paid: bool,
}

impl Default for MintInfo {
    fn default() -> Self {
        Self {
            bump: Default::default(),
            fee_paid: Default::default(),
        }
    }
}

#[derive(Accounts)]
#[instruction(bump: u8)]
pub struct InitMintInfo<'info> {
    #[account(signer)]
    payer: AccountInfo<'info>,
    #[account(
        init,
        payer = payer,
        seeds = [
            mint.key().as_ref(),
        ],
        bump = bump
    )]
    mint_info: ProgramAccount<'info, MintInfo>,
    mint: Account<'info, Mint>,
    config: ProgramAccount<'info, Config>,

    system_program: Program<'info, System>,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct CreateLockerArgs {
    amount: u64,
    unlock_date: i64,
    country_code: String,
    start_emission: Option<i64>,
    vault_bump: u8,
    fee_in_sol: bool,
}

#[derive(Accounts)]
#[instruction(args: CreateLockerArgs)]
pub struct CreateLocker<'info> {
    #[account(
        init,
        payer = creator,
        space = Locker::LEN,
    )]
    locker: ProgramAccount<'info, Locker>,
    #[account(signer)]
    creator: AccountInfo<'info>,
    owner: AccountInfo<'info>,
    #[account(signer)]
    funding_wallet_authority: AccountInfo<'info>,
    #[account(mut)]
    funding_wallet: Account<'info, TokenAccount>,
    #[account(
        seeds = [
            locker.key().as_ref()
        ],
        bump = args.vault_bump
    )]
    vault_authority: AccountInfo<'info>,
    #[account(
        mut,
        constraint = vault.mint == funding_wallet.mint,
        constraint = vault.owner == vault_authority.key()
    )]
    vault: Account<'info, TokenAccount>,
    #[account(mut)]
    fee_wallet: AccountInfo<'info>,
    #[account(mut)]
    fee_token_wallet: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [
            vault.mint.key().as_ref()
        ],
        bump = mint_info.bump
    )]
    mint_info: ProgramAccount<'info, MintInfo>,
    config: ProgramAccount<'info, Config>,
    #[account(
        constraint = country_banlist.key() == config.country_list
    )]
    country_banlist: Account<'info, country_list::CountryBanList>,

    clock: Sysvar<'info, Clock>,
    system_program: Program<'info, System>,
    token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Relock<'info> {
    #[account(mut)]
    locker: ProgramAccount<'info, Locker>,
    #[account(
        signer,
        constraint = locker.owner == owner.key()
    )]
    owner: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct TransferOwnership<'info> {
    #[account(mut)]
    locker: ProgramAccount<'info, Locker>,
    #[account(
        signer,
        constraint = locker.owner == owner.key()
    )]
    owner: AccountInfo<'info>,
    new_owner: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct IncrementLock<'info> {
    #[account(mut)]
    locker: ProgramAccount<'info, Locker>,
    #[account(
        mut,
        constraint = vault.mint == funding_wallet.mint,
        constraint = locker.vault == vault.key()
    )]
    vault: Account<'info, TokenAccount>,
    #[account(
        seeds = [
            vault.mint.key().as_ref()
        ],
        bump = mint_info.bump
    )]
    mint_info: ProgramAccount<'info, MintInfo>,
    #[account(signer)]
    funding_wallet_authority: AccountInfo<'info>,
    #[account(mut)]
    funding_wallet: Account<'info, TokenAccount>,
    #[account(mut)]
    fee_wallet: Account<'info, TokenAccount>,
    config: ProgramAccount<'info, Config>,

    token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawFunds<'info> {
    #[account(mut)]
    locker: ProgramAccount<'info, Locker>,
    #[account(
        signer,
        constraint = locker.owner == owner.key()
    )]
    owner: AccountInfo<'info>,
    /// This authority allows the program to sign token transfer
    /// back to target wallet.
    vault_authority: AccountInfo<'info>,
    #[account(
        mut,
        constraint = vault.owner == vault_authority.key()
    )]
    vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = target_wallet.mint == vault.mint
    )]
    target_wallet: Account<'info, TokenAccount>,

    clock: Sysvar<'info, Clock>,
    token_program: Program<'info, Token>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct SplitLockerArgs {
    locker_bump: u8,
    vault_bump: u8,
    amount: u64,
}

#[derive(Accounts)]
#[instruction(args: SplitLockerArgs)]
pub struct SplitLocker<'info> {
    #[account(mut)]
    old_locker: ProgramAccount<'info, Locker>,
    #[account(
        signer,
        constraint = old_locker.owner == old_owner.key()
    )]
    old_owner: AccountInfo<'info>,
    /// This authority allows the program to sign token transfer
    /// back to target wallet.
    old_vault_authority: AccountInfo<'info>,
    #[account(
        mut,
        constraint = old_vault.owner == old_vault_authority.key()
    )]
    old_vault: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = old_owner,
        seeds = [
            old_locker.key().as_ref(),
            old_locker.current_unlock_date.to_be_bytes().as_ref(),
            args.amount.to_be_bytes().as_ref()
        ],
        bump = args.locker_bump,
        space = Locker::LEN,
    )]
    new_locker: ProgramAccount<'info, Locker>,
    new_owner: AccountInfo<'info>,
    /// This authority allows the program to sign token transfer
    /// back to target wallet.
    #[account(
        seeds = [
            new_locker.key().as_ref()
        ],
        bump = args.vault_bump
    )]
    new_vault_authority: AccountInfo<'info>,
    #[account(
        mut,
        constraint = new_vault.mint == old_vault.mint
    )]
    new_vault: Account<'info, TokenAccount>,

    token_program: Program<'info, Token>,
    system_program: Program<'info, System>,
}

/// For test purposes only!
#[derive(Accounts)]
pub struct CloseLocker<'info> {
    #[account(mut)]
    locker: ProgramAccount<'info, Locker>,
    #[account(
        signer,
        constraint = locker.owner == owner.key()
    )]
    owner: AccountInfo<'info>,
    /// This authority allows the program to sign token transfer
    /// back to target wallet.
    vault_authority: AccountInfo<'info>,
    #[account(
        mut,
        constraint = vault.owner == vault_authority.key()
    )]
    vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = target_wallet.mint == vault.mint
    )]
    target_wallet: Account<'info, TokenAccount>,

    token_program: Program<'info, Token>,
}

/// floor(a * b / denominator)
pub fn mul_div<SrcA, SrcB, SrcD>(a: SrcA, b: SrcB, denominator: SrcD) -> Option<u64>
where
    SrcA: fixed::traits::ToFixed,
    SrcB: fixed::traits::ToFixed,
    SrcD: fixed::traits::ToFixed,
{
    use fixed::types::U64F64;

    let a = U64F64::from_num(a);
    let b = U64F64::from_num(b);
    let denominator = U64F64::from_num(denominator);

    a.checked_mul(b)
        .and_then(|r| r.checked_div(denominator))
        .and_then(|r| r.floor().checked_as::<u64>())
}

fn should_pay_in_sol(config: &Config, mint_info: &MintInfo, fee_in_sol: bool) -> bool {
    match (
        config.mint_info_permissioned,
        fee_in_sol,
        mint_info.fee_paid,
    ) {
        // always paying
        (true, _, _) => true,
        // pay if pay in sol is chosen but no fee paid yet
        (_, true, false) => true,
        // do not pay in other cases
        (_, _, _) => false,
    }
}

fn should_pay_in_tokens(config: &Config, mint_info: &MintInfo, fee_in_sol: bool) -> bool {
    match (
        config.mint_info_permissioned,
        fee_in_sol,
        mint_info.fee_paid,
    ) {
        // always paying
        (true, _, _) => true,
        // pay if pay in sol is not chosen but no fee paid yet
        (_, false, false) => true,
        // do not pay in other cases
        (_, _, _) => false,
    }
}

struct FeeInSol<'pay, 'info> {
    fee_wallet: &'pay AccountInfo<'info>,
    payer: &'pay AccountInfo<'info>,
    config: &'pay Config,
    mint_info: &'pay mut MintInfo,
    system_program: &'pay Program<'info, System>,
}

impl FeeInSol<'_, '_> {
    fn pay(self) -> Result<()> {
        require!(
            self.fee_wallet.key() == self.config.fee_wallet,
            InvalidFeeWallet
        );

        self.payer.key().log();
        self.fee_wallet.key().log();

        solana_program::program::invoke(
            &solana_program::system_instruction::transfer(
                self.payer.to_account_info().key,
                self.fee_wallet.key,
                self.config.fee_in_sol * solana_program::native_token::LAMPORTS_PER_SOL,
            ),
            &[
                self.payer.to_account_info(),
                self.fee_wallet.to_account_info(),
                self.system_program.to_account_info(),
            ],
        )?;

        // if not permissioned we allow one-time fees
        if !self.config.mint_info_permissioned {
            self.mint_info.fee_paid = true;
        }

        Ok(())
    }
}

struct FeeInTokens<'pay, 'info> {
    config: &'pay Config,
    funding_wallet: &'pay mut Account<'info, TokenAccount>,
    funding_wallet_authority: &'pay AccountInfo<'info>,
    fee_wallet: &'pay Account<'info, TokenAccount>,
    amount: u64,
    token_program: &'pay Program<'info, Token>,
}

impl FeeInTokens<'_, '_> {
    fn pay(self) -> Result<u64> {
        let associated_token_account =
            get_associated_token_address(&self.config.fee_wallet, &self.funding_wallet.mint);

        require!(
            associated_token_account == self.fee_wallet.key(),
            InvalidFeeWallet
        );

        let lock_fee = mul_div(
            self.amount,
            self.config.fee_in_token_numerator,
            self.config.fee_in_token_denominator,
        )
        .ok_or(ErrorCode::IntegerOverflow)?;

        TokenTransfer {
            amount: lock_fee,
            from: self.funding_wallet,
            to: self.fee_wallet,
            authority: self.funding_wallet_authority,
            token_program: self.token_program,
            signers: None,
        }
        .make()?;

        sol_log_64(self.amount, lock_fee, self.amount - lock_fee, 0, 0);

        Ok(lock_fee)
    }
}

struct TokenTransfer<'pay, 'info> {
    amount: u64,
    from: &'pay mut Account<'info, TokenAccount>,
    to: &'pay Account<'info, TokenAccount>,
    authority: &'pay AccountInfo<'info>,
    token_program: &'pay Program<'info, Token>,
    signers: Option<&'pay [&'pay [&'pay [u8]]]>,
}

impl TokenTransfer<'_, '_> {
    fn make(self) -> Result<()> {
        let amount_before = self.from.amount;

        self.from.key().log();
        self.to.key().log();
        self.authority.key().log();

        let cpi_ctx = CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.from.to_account_info(),
                to: self.to.to_account_info(),
                authority: self.authority.to_account_info(),
            },
        );
        let cpi_ctx = match self.signers {
            Some(signers) => cpi_ctx.with_signer(signers),
            None => cpi_ctx,
        };

        token::transfer(cpi_ctx, self.amount)?;

        self.from.reload()?;
        let amount_after = self.from.amount;

        sol_log_64(amount_before, amount_after, self.amount, 0, 0);

        require!(
            amount_before - amount_after == self.amount,
            InvalidAmountTransferred
        );

        Ok(())
    }
}
