use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program_error::ProgramError,
    msg,
    pubkey::Pubkey,
    program_pack::{Pack, IsInitialized},
    sysvar::{rent::Rent, Sysvar},
    program::invoke
};

use crate::{instruction::EscrowInstruction, error::EscrowError, state::Escrow};

pub struct Processor;

impl Processor {
    pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], instruction_data: &[u8]) -> ProgramResult {
        let instruction = EscrowInstruction::unpack(instruction_data)?; // uses the unpack function defined in instruction, the ? will work to either give the value if it is ok, or call the error if there is one

        match instruction { // here we include code that will be called depending on the instruction given
            EscrowInstruction::InitEscrow { amount } => {
                msg!("Instruction: InitEscrow");
                Self::process_init_escrow(accounts, amount, program_id)
            }
        }
    }
    
    fn process_init_escrow(
        accounts: &[AccountInfo],
        amount: u64,
        program_id: &Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter(); // mut makes this accounts iterable mutable, which we need to extract elements from it
        let initializer = next_account_info(account_info_iter)?; // this creates an iterator on the accounts, so the first iteration will return the initializer.

        if !initializer.is_signer {  // the initializer needs to be a signer otherwise the transaction won't work, so check for that as so
            return Err(ProgramError::MissingRequiredSignature);
        }

        let temp_token_account = next_account_info(account_info_iter)?;

        let token_to_receive_account = next_account_info(account_info_iter)?;
        if *token_to_receive_account.owner != spl_token::id() { // this checks whether the owner of the token_to_receive account is the token program 
            return Err(ProgramError::IncorrectProgramId);
        }

        let escrow_account = next_account_info(account_info_iter)?;
        let rent = &Rent::from_account_info(next_account_info(account_info_iter)?)?;

        if !rent.is_exempt(escrow_account.lamports(), escrow_account.data_len()) {
            return Err(EscrowError::NotRentExempt.into());
        }

        let mut escrow_info = Escrow::unpack_unchecked(&escrow_account.data.borrow())?; // here we are accessing the data field of the escrow account - this is a u8 array that we need to deserialize with an unpacking function
        if escrow_info.is_initialized() {
            return Err(ProgramError::AccountAlreadyInitialized);
        }
        
        escrow_info.is_initialized = true;
        escrow_info.initializer_pubkey = *initializer.key;
        escrow_info.temp_token_account_pubkey = *temp_token_account.key;
        escrow_info.initializer_token_to_receive_account_pubkey = *token_to_receive_account.key;
        escrow_info.expected_amount = amount;

        Escrow::pack(escrow_info, &mut escrow_account.data.borrow_mut())?; // pack is an internal function that calls our pack_into_slice function from state.rs
        let (pda, _bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id);

        let token_program = next_account_info(account_info_iter)?;
        let owner_change_ix = spl_token::instruction::set_authority(
            token_program.key, // token program id
            temp_token_account.key, // the account whose authority we would like to change
            Some(&pda), // the account that is the new authority (the PDA)
            spl_token::instruction::AuthorityType::AccountOwner, // the type of authority change (owner change)
            initializer.key, // the current account owner
            &[&initializer.key], // the public key to sign the CPI (cross program invocation)
        )?;
        
        msg!("Calling the token program to transfer token account ownership...");
        invoke(
            &owner_change_ix,
            &[
                temp_token_account.clone(),
                initializer.clone(),
                token_program.clone(),
            ],
        )?;

        Ok(())
    }
}

