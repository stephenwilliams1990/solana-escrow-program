use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program_error::ProgramError,
    msg,
    pubkey::Pubkey,
    program_pack::{Pack, IsInitialized},
    sysvar::{rent::Rent, Sysvar},
    program::{invoke, invoke_signed}
};

use spl_token::state::Account as TokenAccount;

use crate::{instruction::EscrowInstruction, error::EscrowError, state::Escrow};

pub struct Processor;

impl Processor {
    pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], instruction_data: &[u8]) -> ProgramResult {
        let instruction = EscrowInstruction::unpack(instruction_data)?; // uses the unpack function defined in instruction, the ? will work to either give the value if it is ok, or call the error if there is one

        match instruction { // here we include code that will be called depending on the instruction given
            EscrowInstruction::InitEscrow { amount } => {
                msg!("Instruction: InitEscrow");
                Self::process_init_escrow(accounts, amount, program_id)
            },
            EscrowInstruction::Exchange { amount } => {
                msg!("Instruction: Exchange");
                Self::process_exchange(accounts, amount, program_id)
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
        let rent = &Rent::from_account_info(next_account_info(account_info_iter)?)?; // rent should be able to be taken from sysvars in new versions 

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
        let (pda, _bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id); // we place the _ before the variable as we will intentionally not use that for now

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

    fn process_exchange(
        accounts: &[AccountInfo],
        amount_expected_by_taker: u64,
        program_id: &Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter(); 
        let taker = next_account_info(account_info_iter)?; // Bob's account

        if !taker.is_signer {  // the initializer needs to be a signer otherwise the transaction won't work, so check for that as so
            return Err(ProgramError::MissingRequiredSignature);
        }

        let send_token_account = next_account_info(account_info_iter)?; // takers token account for the token they will send

        //// !!! need to put in a check that this pubKey is equal to the info in the escrow account later

        let receive_token_account = next_account_info(account_info_iter)?; // takers token account for the token they will receive

        //// !!! need to check that this is equal to the temp account owned by the PDA

        let pdas_temp_token_account = next_account_info(account_info_iter)?;

        let pdas_temp_token_account_info = TokenAccount::unpack(&pdas_temp_token_account.data.borrow())?; // this part I don't get
        let (pda, bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id); // we place the _ before the variable as we will intentionally not use that for now

        if amount_expected_by_taker != pdas_temp_token_account_info.amount {
            return Err(EscrowError::ExpectedAmountMismatch.into());
        }

        let initializers_main_account = next_account_info(account_info_iter)?;
        let initializer_token_to_receive_account = next_account_info(account_info_iter)?;
        let escrow_account = next_account_info(account_info_iter)?;

        let escrow_info = Escrow::unpack(&escrow_account.data.borrow())?;

        if escrow_info.temp_token_account_pubkey != *pdas_temp_token_account.key {
            return Err(ProgramError::InvalidAccountData);
        }

        if escrow_info.initializer_pubkey != *initializers_main_account.key {
            return Err(ProgramError::InvalidAccountData);
        }

        if escrow_info.initializer_token_to_receive_account_pubkey != *initializer_token_to_receive_account.key {
            return Err(ProgramError::InvalidAccountData);
        }

        let token_program = next_account_info(account_info_iter)?;

        let transfer_to_initializer_ix =  spl_token::instruction::transfer(
            token_program.key,
            send_token_account.key,
            initializer_token_to_receive_account.key,
            taker.key,
            &[&taker.key],
            escrow_info.expected_amount,
        )?;
        msg!("Calling the token program to transfer tokens to the escrow's initializer...");
        invoke(
            &transfer_to_initializer_ix,
            &[
                send_token_account.clone(),
                initializer_token_to_receive_account.clone(),
                taker.clone(),
                token_program.clone(),
            ]
        )?;

        let pda_account = next_account_info(account_info_iter)?;
        
        let transfer_to_taker_ix = spl_token::instruction::transfer(
            token_program.key,
            pdas_temp_token_account.key,
            receive_token_account.key,
            &pda, // done like this as pda is the key, not the keypair
            &[&pda],
            pdas_temp_token_account_info.amount, // check if this works should be the same as the amount in the pdas_temp_token_account_info
        )?;
        msg!("Calling the token program to transfer tokens to the taker..");
        invoke_signed(
            &transfer_to_taker_ix,
            &[
                pdas_temp_token_account.clone(),
                receive_token_account.clone(),
                pda_account.clone(), // note that this is the pda account not the pda address that was generate with the b"escrow" seed
                token_program.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]], // why so many []? - this is in the Calling Between Programs Solana docs under cross program invocations still don't get the b"escrow"[..]
        )?;

        let close_pda_temp_token_account_ix = spl_token::instruction::close_account(
            token_program.key,
            pdas_temp_token_account.key,
            initializers_main_account.key,
            &pda,
            &[&pda],
        )?;
        msg!("Calling the token program to close pda's temp account...");
        invoke_signed(
            &close_pda_temp_token_account_ix,
            &[
                pdas_temp_token_account.clone(),
                initializers_main_account.clone(),
                pda_account.clone(), // note that this is the pda account not the pda address that was generate with the b"escrow" seed
                token_program.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]], 
        )?;

        // add the rent back to Alice's account and clear the data in the escrow account
        msg!("Closing the escrow account...");
        **initializers_main_account.lamports.borrow_mut() = initializers_main_account.lamports()
        .checked_add(escrow_account.lamports())
        .ok_or(EscrowError::AmountOverflow)?;
        **escrow_account.lamports.borrow_mut() = 0;
        *escrow_account.data.borrow_mut() = &mut [];

        Ok(())
    }
}   

