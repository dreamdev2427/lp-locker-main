const spl = require("@solana/spl-token");
const web3 = require("@solana/web3.js");
const anchor = require('@project-serum/anchor');
const TokenInstructions = require("@project-serum/serum").TokenInstructions;
const serumCmn = require('@project-serum/common');

const TOKEN_PROGRAM_ID = new anchor.web3.PublicKey(
  TokenInstructions.TOKEN_PROGRAM_ID.toString()
);

async function createTokenAccount(provider, mint, owner) {
  if (owner === undefined) {
    owner = provider.wallet.publicKey;
  }
  // Allocate memory for the account
  const balanceNeeded = await spl.Token.getMinBalanceRentForExemptAccount(
    provider.connection,
  );

  const seed = mint.toString() + owner.toString();

  const tokenAccount = await web3.PublicKey.createWithSeed(
    provider.wallet.publicKey,
    seed,
    TOKEN_PROGRAM_ID
  );
  console.log(tokenAccount);

  const tx = new web3.Transaction();
  tx.add(
    web3.SystemProgram.createAccountWithSeed({
      fromPubkey: provider.wallet.publicKey,
      newAccountPubkey: tokenAccount,
      basePubkey: provider.wallet.publicKey,
      seed,
      lamports: balanceNeeded,
      space: spl.AccountLayout.span,
      programId: TOKEN_PROGRAM_ID,
    }),
  );

  tx.add(
    spl.Token.createInitAccountInstruction(
      TOKEN_PROGRAM_ID,
      mint,
      tokenAccount,
      owner,
    ),
  );

  await provider.send(tx);

  return tokenAccount;
}

const FAILED_TO_FIND_ACCOUNT = 'Failed to find token account';
const INVALID_ACCOUNT_OWNER = 'Invalid account owner';

async function getOrCreateAssociatedTokenAccountInstrs(provider, mint, owner) {
  let associatedTokenAddress = await anchor.utils.token.associatedAddress({ mint, owner });

  try {
    const _ = await serumCmn.getTokenAccount(provider, associatedTokenAddress);
    return [associatedTokenAddress, []];
  } catch (err) {
    // INVALID_ACCOUNT_OWNER can be possible if the associatedAddress has
    // already been received some lamports (= became system accounts).
    // Assuming program derived addressing is safe, this is the only case
    // for the INVALID_ACCOUNT_OWNER in this code-path
    if (
      err.message === FAILED_TO_FIND_ACCOUNT ||
      err.message === INVALID_ACCOUNT_OWNER
    ) {
      let createTokenAccountInstr = spl.Token.createAssociatedTokenAccountInstruction(
        spl.ASSOCIATED_TOKEN_PROGRAM_ID,
        TOKEN_PROGRAM_ID,
        mint,
        associatedTokenAddress,
        owner,
        provider.wallet.publicKey,
      );
      return [associatedTokenAddress, [createTokenAccountInstr]];
    } else {
      throw err;
    }
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

module.exports = {
  createTokenAccount,
  getOrCreateAssociatedTokenAccountInstrs,
  sleep,
  TOKEN_PROGRAM_ID,
};
