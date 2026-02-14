use borsh::{BorshDeserialize, BorshSerialize};

pub const STANDARD_AMM_SWAP_BASE_INPUT: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222];
pub const RAYDIUM_STANDARD_AMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
pub const RAYDIUM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const WSOL_ADDRESS: &str = "So11111111111111111111111111111111111111112";

#[derive(BorshSerialize, BorshDeserialize)]
pub struct SwapInstructionBaseIn {
    pub discriminator: u8,
    pub amount_in: u64,
    pub minimum_amount_out: u64,
}
