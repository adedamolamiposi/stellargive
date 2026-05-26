#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, String,
    Symbol,
};

#[contract]
pub struct StellarGiveContract;

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum CampaignStatus {
    Active,
    Funded,
    Claimed,
    Expired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CreatedEvent {
    pub id: u64,
    pub creator: Address,
    pub target_amount: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Campaign {
    pub id: u64,
    pub creator: Address,
    pub beneficiary: Address,
    pub title: String,
    pub target_amount: i128,
    pub raised_amount: i128,
    pub deadline: u64,
    pub accepted_token: Address,
    pub status: CampaignStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracterror]
#[repr(u32)]
pub enum ContractError {
    Unauthorized = 1,
    InvalidDeadline = 2,
    InvalidAmount = 3,
    CampaignNotFound = 4,
    InvalidToken = 5,
    CampaignNotActive = 6,
    ClaimNotAllowed = 7,
    AlreadyClaimed = 8,
    ReentrancyDetected = 9,
    EmptyTitle = 10,
    NothingToClaim = 11,
}

fn next_id_key() -> Symbol {
    symbol_short!("NEXT")
}

fn lock_key() -> Symbol {
    symbol_short!("LOCK")
}

fn campaign_key(id: u64) -> (Symbol, u64) {
    (symbol_short!("CMP"), id)
}

fn read_next_id(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&next_id_key())
        .unwrap_or(1_u64)
}

fn write_next_id(env: &Env, next_id: u64) {
    env.storage().persistent().set(&next_id_key(), &next_id);
}

fn read_campaign(env: &Env, id: u64) -> Result<Campaign, ContractError> {
    env.storage()
        .persistent()
        .get(&campaign_key(id))
        .ok_or(ContractError::CampaignNotFound)
}

fn write_campaign(env: &Env, campaign: &Campaign) {
    env.storage()
        .persistent()
        .set(&campaign_key(campaign.id), campaign);
}

fn enter_lock(env: &Env) -> Result<(), ContractError> {
    let key = lock_key();
    // Temporary lock prevents re-entrant state changes in the same execution context.
    if env.storage().temporary().get::<_, bool>(&key).unwrap_or(false) {
        return Err(ContractError::ReentrancyDetected);
    }
    env.storage().temporary().set(&key, &true);
    Ok(())
}

fn exit_lock(env: &Env) {
    env.storage().temporary().remove(&lock_key());
}

fn derive_status(now: u64, campaign: &Campaign) -> CampaignStatus {
    // Claimed is terminal and must not be downgraded by timestamp checks.
    if campaign.status == CampaignStatus::Claimed {
        return CampaignStatus::Claimed;
    }

    if campaign.raised_amount >= campaign.target_amount {
        return CampaignStatus::Funded;
    }

    if now > campaign.deadline {
        return CampaignStatus::Expired;
    }

    CampaignStatus::Active
}

fn sync_status(env: &Env, campaign: &mut Campaign) {
    let updated = derive_status(env.ledger().timestamp(), campaign);
    if updated != campaign.status {
        campaign.status = updated;
        write_campaign(env, campaign);
    }
}

fn validate_token_contract(env: &Env, token_address: &Address) -> Result<(), ContractError> {
    // Validate token interface by calling a standard SEP-41 read method.
    if token::TokenClient::new(env, token_address)
        .try_decimals()
        .is_err()
    {
        return Err(ContractError::InvalidToken);
    }
    Ok(())
}

#[contractimpl]
impl StellarGiveContract {
    pub fn create_campaign(
        env: Env,
        creator: Address,
        beneficiary: Address,
        title: String,
        target_amount: i128,
        deadline: u64,
        accepted_token: Address,
    ) -> Result<u64, ContractError> {
        creator.require_auth();

        if title.len() == 0 {
            return Err(ContractError::EmptyTitle);
        }
        if target_amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        if deadline <= env.ledger().timestamp() {
            return Err(ContractError::InvalidDeadline);
        }
        validate_token_contract(&env, &accepted_token)?;

        let id = read_next_id(&env);
        let next_id = id.checked_add(1).ok_or(ContractError::InvalidAmount)?;
        write_next_id(&env, next_id);

        let campaign = Campaign {
            id,
            creator: creator.clone(),
            beneficiary: beneficiary.clone(),
            title,
            target_amount,
            raised_amount: 0,
            deadline,
            accepted_token: accepted_token.clone(),
            status: CampaignStatus::Active,
        };

        write_campaign(&env, &campaign);
        env.events().publish(
            (symbol_short!("created"),),
            CreatedEvent {
                id,
                creator,
                target_amount: campaign.target_amount,
            },
        );

        Ok(id)
    }

    pub fn donate(
        env: Env,
        donor: Address,
        campaign_id: u64,
        amount: i128,
    ) -> Result<(), ContractError> {
        donor.require_auth();
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        enter_lock(&env)?;
        let result = (|| {
            let mut campaign = read_campaign(&env, campaign_id)?;
            sync_status(&env, &mut campaign);

            if campaign.status != CampaignStatus::Active {
                return Err(ContractError::CampaignNotActive);
            }

            token::TokenClient::new(&env, &campaign.accepted_token).transfer(
                &donor,
                &env.current_contract_address(),
                &amount,
            );

            campaign.raised_amount = campaign
                .raised_amount
                .checked_add(amount)
                .ok_or(ContractError::InvalidAmount)?;

            campaign.status = if campaign.raised_amount >= campaign.target_amount {
                CampaignStatus::Funded
            } else {
                CampaignStatus::Active
            };

            write_campaign(&env, &campaign);
            env.events().publish(
                (symbol_short!("donation"), symbol_short!("received")),
                (
                    campaign.id,
                    donor,
                    amount,
                    campaign.raised_amount,
                    campaign.accepted_token.clone(),
                ),
            );
            Ok(())
        })();

        exit_lock(&env);
        result
    }

    pub fn claim_funds(env: Env, caller: Address, campaign_id: u64) -> Result<i128, ContractError> {
        let mut campaign = read_campaign(&env, campaign_id)?;
        sync_status(&env, &mut campaign);

        if campaign.status == CampaignStatus::Claimed {
            return Err(ContractError::AlreadyClaimed);
        }

        if caller != campaign.creator && caller != campaign.beneficiary {
            return Err(ContractError::Unauthorized);
        }
        caller.require_auth();

        let now = env.ledger().timestamp();
        let can_claim = campaign.raised_amount >= campaign.target_amount || now > campaign.deadline;
        if !can_claim {
            return Err(ContractError::ClaimNotAllowed);
        }
        if campaign.raised_amount <= 0 {
            return Err(ContractError::NothingToClaim);
        }

        enter_lock(&env)?;
        let result = (|| {
            let amount = campaign.raised_amount;
            // Funds are always paid out to beneficiary to keep payout path deterministic.
            token::TokenClient::new(&env, &campaign.accepted_token).transfer(
                &env.current_contract_address(),
                &campaign.beneficiary,
                &amount,
            );

            campaign.raised_amount = 0;
            campaign.status = CampaignStatus::Claimed;
            write_campaign(&env, &campaign);

            env.events().publish(
                (symbol_short!("funds"), symbol_short!("claimed")),
                (campaign.id, caller, campaign.beneficiary, amount, campaign.accepted_token),
            );

            Ok(amount)
        })();

        exit_lock(&env);
        result
    }

    pub fn get_campaign(env: Env, campaign_id: u64) -> Result<Campaign, ContractError> {
        let mut campaign = read_campaign(&env, campaign_id)?;
        campaign.status = derive_status(env.ledger().timestamp(), &campaign);
        Ok(campaign)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
    use soroban_sdk::{token, Address, Env, String, TryFromVal};

    fn set_timestamp(env: &Env, timestamp: u64) {
        let mut ledger = env.ledger().get();
        ledger.timestamp = timestamp;
        env.ledger().set(ledger);
    }

    fn setup() -> (
        Env,
        StellarGiveContractClient<'static>,
        Address,
        Address,
        Address,
        token::Client<'static>,
        token::StellarAssetClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let creator = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let donor = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
        let token_client = token::Client::new(&env, &token_id.address());
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id.address());

        token_admin_client.mint(&donor, &1_000_000);
        token_admin_client.mint(&creator, &1_000_000);

        let contract_id = env.register_contract(None, StellarGiveContract);
        let client = StellarGiveContractClient::new(&env, &contract_id);

        (env, client, creator, beneficiary, donor, token_client, token_admin_client)
    }

    #[test]
    fn create_and_get_campaign() {
        let (env, client, creator, beneficiary, _donor, token_client, _) = setup();
        set_timestamp(&env, 1_000);

        let id = client.create_campaign(
            &creator,
            &beneficiary,
            &String::from_str(&env, "Flood Relief"),
            &500_000,
            &2_000,
            &token_client.address,
        );

        let campaign = client.get_campaign(&id);
        assert_eq!(campaign.id, 1);
        assert_eq!(campaign.status, CampaignStatus::Active);
        assert_eq!(campaign.creator, creator);
        assert_eq!(campaign.beneficiary, beneficiary);
        assert_eq!(campaign.target_amount, 500_000);
        assert_eq!(campaign.raised_amount, 0);
    }

    #[test]
    fn create_campaign_emits_created_event() {
        let (env, client, creator, beneficiary, _donor, token_client, _) = setup();
        set_timestamp(&env, 1_000);

        let target_amount: i128 = 500_000;
        let id = client.create_campaign(
            &creator,
            &beneficiary,
            &String::from_str(&env, "Flood Relief"),
            &target_amount,
            &2_000,
            &token_client.address,
        );

        let event = env
            .events()
            .all()
            .iter()
            .find(|(addr, topics, _)| {
                addr == &client.address
                    && topics
                        .get(0)
                        .and_then(|t| Symbol::try_from_val(&env, &t).ok())
                        == Some(symbol_short!("created"))
            })
            .expect("CreatedEvent was not emitted by create_campaign");

        let payload = CreatedEvent::try_from_val(&env, &event.2)
            .expect("event data did not decode as CreatedEvent");
        assert_eq!(payload.id, id);
        assert_eq!(payload.creator, creator);
        assert_eq!(payload.target_amount, target_amount);
    }

    #[test]
    fn donate_updates_raised_and_status() {
        let (env, client, creator, beneficiary, donor, token_client, _) = setup();
        set_timestamp(&env, 5_000);

        let campaign_id = client.create_campaign(
            &creator,
            &beneficiary,
            &String::from_str(&env, "Medical Aid"),
            &100_000,
            &10_000,
            &token_client.address,
        );

        client.donate(&donor, &campaign_id, &40_000);
        let campaign_after_first = client.get_campaign(&campaign_id);
        assert_eq!(campaign_after_first.raised_amount, 40_000);
        assert_eq!(campaign_after_first.status, CampaignStatus::Active);

        client.donate(&donor, &campaign_id, &60_000);
        let campaign_after_second = client.get_campaign(&campaign_id);
        assert_eq!(campaign_after_second.raised_amount, 100_000);
        assert_eq!(campaign_after_second.status, CampaignStatus::Funded);
    }

    #[test]
    fn claim_when_target_met_transfers_to_beneficiary() {
        let (env, client, creator, beneficiary, donor, token_client, _) = setup();
        set_timestamp(&env, 10_000);

        let campaign_id = client.create_campaign(
            &creator,
            &beneficiary,
            &String::from_str(&env, "School Rebuild"),
            &120_000,
            &20_000,
            &token_client.address,
        );

        client.donate(&donor, &campaign_id, &120_000);

        let beneficiary_before = token_client.balance(&beneficiary);
        let claimed = client.claim_funds(&creator, &campaign_id);
        let beneficiary_after = token_client.balance(&beneficiary);
        let campaign = client.get_campaign(&campaign_id);

        assert_eq!(claimed, 120_000);
        assert_eq!(beneficiary_after - beneficiary_before, 120_000);
        assert_eq!(campaign.status, CampaignStatus::Claimed);
        assert_eq!(campaign.raised_amount, 0);
    }

    #[test]
    fn claim_after_deadline_when_target_not_met() {
        let (env, client, creator, beneficiary, donor, token_client, _) = setup();
        set_timestamp(&env, 100);

        let campaign_id = client.create_campaign(
            &creator,
            &beneficiary,
            &String::from_str(&env, "Emergency Shelter"),
            &500_000,
            &500,
            &token_client.address,
        );

        client.donate(&donor, &campaign_id, &50_000);
        set_timestamp(&env, 600);

        let claimed = client.claim_funds(&beneficiary, &campaign_id);
        let campaign = client.get_campaign(&campaign_id);

        assert_eq!(claimed, 50_000);
        assert_eq!(campaign.status, CampaignStatus::Claimed);
    }

    #[test]
    fn unauthorized_claim_fails() {
        let (env, client, creator, beneficiary, donor, token_client, _) = setup();
        set_timestamp(&env, 200);

        let campaign_id = client.create_campaign(
            &creator,
            &beneficiary,
            &String::from_str(&env, "Food Support"),
            &100_000,
            &1_000,
            &token_client.address,
        );
        client.donate(&donor, &campaign_id, &10_000);
        set_timestamp(&env, 1_100);

        let attacker = Address::generate(&env);
        let error = client.try_claim_funds(&attacker, &campaign_id);
        assert!(error.is_err());
    }
}
