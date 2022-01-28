use anchor_lang::prelude::*;

declare_id!("DKRoqjR3xeYnp9WtiTgjyFDRwo2L1SBQcYsp6hdUU5Tw");

#[error]
pub enum ErrorCode {
    UnknownCountry,
}

#[program]
pub mod country_list {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, countries: Vec<[u8; 2]>) -> Result<()> {
        let country_banlist = &mut ctx.accounts.country_banlist;

        for code in countries.iter() {
            country_banlist.countries.push(CountryData {
                code: *code,
                banned: false,
            });
        }
        country_banlist.countries.dedup_by_key(|c| c.code);
        country_banlist.countries.sort_by_key(|c| c.code);

        country_banlist.admin = ctx.accounts.admin.key();

        Ok(())
    }

    pub fn flip_ban(ctx: Context<FlipBan>, country: String, value: bool) -> Result<()> {
        let country_banlist = &mut ctx.accounts.country_banlist;

        let array = string_to_byte_array(&country);
        let maybe_idx = country_banlist
            .countries
            // so we need to sort countries on initialization
            .binary_search_by_key(&array, |c| c.code);

        match maybe_idx {
            Ok(idx) => {
                // unwrap since we have found index of the entry
                let country_ban = country_banlist.countries.get_mut(idx).unwrap();
                country_ban.banned = value;
            }
            Err(_) => {
                return Err(ErrorCode::UnknownCountry.into());
            }
        }

        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CountryData {
    pub code: [u8; 2],
    banned: bool,
}

#[account]
#[derive(Debug)]
pub struct CountryBanList {
    pub countries: Vec<CountryData>,
    admin: Pubkey,
}

impl CountryBanList {
    pub const MAX_COUNTRIES: usize = 256;
    // 8 -- discriminator
    pub const LEN: usize = std::mem::size_of::<Pubkey>()
        + Self::MAX_COUNTRIES * std::mem::size_of::<CountryData>()
        + 8;

    pub fn is_country_valid(&self, country: &str) -> bool {
        let array = string_to_byte_array(&country);
        let maybe_country = self.countries.iter().find(|c| c.code == array);
        match maybe_country {
            Some(country) => !country.banned,
            None => false,
        }
    }
}

impl Default for CountryBanList {
    fn default() -> Self {
        Self {
            countries: Default::default(),
            admin: Default::default(),
        }
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = admin,
        space = CountryBanList::LEN
    )]
    country_banlist: ProgramAccount<'info, CountryBanList>,
    #[account(signer)]
    admin: AccountInfo<'info>,

    system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FlipBan<'info> {
    #[account(mut)]
    country_banlist: ProgramAccount<'info, CountryBanList>,
    #[account(
        signer,
        constraint = admin.key() == country_banlist.admin
    )]
    admin: AccountInfo<'info>,
}

pub fn string_to_byte_array(s: &str) -> [u8; 2] {
    let mut array = [0; 2];
    array.copy_from_slice(&s.as_bytes()[..2]);

    array
}
