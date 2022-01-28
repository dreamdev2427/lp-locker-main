


## Glossary

* *mint* -- SPL token address. It's the address of token (or mint) **itself**.
* *SPL token account* -- SPL token account address. It's the address of concrete
wallet with some SPL tokens.

```bash
# address of some token account
$  spl-token account-info --address 3JAXHLUKqwnDHSaB3eJ5LgtzwRZWD8pDH4s7pqVcMnCe

Address: 3JAXHLUKqwnDHSaB3eJ5LgtzwRZWD8pDH4s7pqVcMnCe # <-- this is token account
Balance: 100
Mint: 7Sh6EPaEuMrd1Acrdb5MRF3Kf4aA6N329FmJeFJbcogy # <-- this is mint
Owner: 9VL5TQfvHsSdCSQdLDWEvTUfUbLdUr8qH2AUsWpU9C4o
State: Initialized
Delegation: (not set)
Close authority: (not set)
```

## Provider and Client setup

```js
// there are packages for vue etc
import { useWallet } from '@solana/wallet-adapter-react';
// this is our client package
import * as locker from 'lp-locker';

// ask user to connect some wallet (Phantom or something else)
const wallet = useWallet();

function getProvider() {
  const opts = {
    preflightCommitment: "processed",
  };
  // change it to https://api.devnet.solana.com to connect
  // to devnet and so on
  const network = "http://127.0.0.1:8899";
  const connection = new Connection(network, opts.preflightCommitment);

  // using provided wallet
  const provider = new Provider(
      connection, wallet, opts.preflightCommitment
  );
  return provider;
}

const provider = getProvider();
// we want to connect to token locker program on devnet
// you can use locker.LP_LOCKER to connect to lp locker program
const client = new locker.Client(provider, locker.TOKEN_LOCKER, locker.DEVNET);
```


## Create Locker

```js
const creator = provider.wallet.publicKey;

await client.createLocker({
    // Date.now() returns timestamp in milliseconds
    // but we need the value in seconds
    unlockDate: new anchor.BN(Date.now() / 1000 + 20),
    countryCode: "RU",
    startEmission: null,
    amount: new anchor.BN(10000),
    creator,
    owner: creator,
    fundingWalletAuthority: creator,
    fundingWallet,
    feeInSol: true,
});
```

`client.createLocker(args)` -- creates locker with specified amount and unlock date.
Returns the address of newly created locker.

> If you use LP locker, you can use only accepted tokens.
> You can check if token is accepted by calling method `isTokenAccepted(mint)`.

* `args`:

```js
{
    // Unix timestamp (seconds!) of type anchor.BN.
    unlockDate,
    // 2 letter country code ("RU", "UK" etc).
    // List of codes in the repo -- Country List.csv
    countryCode,
    // Unix timestamp in seconds of type anchor.BN *or* null.
    // Setting this value allows the users to withdraw funds before
    // unlock date linearly.
    // LP locker should always have null -- program will fail if there's some value.
    startEmission,
    // Amount to lock of type anchor.BN. There will be fee if:
    // * you use LP locker;
    // * you use Token locker and the mint is not whitelisted. You can check
    //   if the mint is whitelisted by calling method `client.isMintWhitelisted(mint)`.
    amount,
    // `anchor.web3.PublicKey` of creator of this locker. It should be
    // a signer too, so it's preferrable to use `provider.wallet.publicKey`.
    creator,
    // `anchor.web3.PublicKey` of locker owner. It can be anyone, so it's
    // not required for creator and owner to be the same account.
    // The signature of owner is not required too.
    // `provider.wallet.publicKey` as `owner` is the simplest case.
    owner,
    // `anchor.web3.PublicKey` of funding wallet owner.
    // It should sign the transaction, so it's better to use
    // `provider.wallet.publicKey`.
    fundingWalletAuthority,
    // `anchor.web.PublicKey` of SPL token account from which
    // the locker will be funded. The amount of tokens you
    // specified earlier will be transferred from this account
    // to some program-controlled vault.
    fundingWallet,
    // `boolean`: if true then fee is paid in SOL,
    // else paid in locked token.
    // If token is already whitelisted it's better to set this to true
    // to avoid any fees.
    feeInSol,
}
```

## Get Lockers

`client.getLockers()` -- returns created lockers.
`client.getLockersOwnerBy(owner)` -- returns lockers owned by specific account.

* `owner` -- account public key

## Relock

`client.relock(unlockDate)` -- relocks the locker to some date that should be
later than the original one.

* `unlockDate` -- new unlock date:
    - should be later than original one;
    - type is anchor.BN;
    - unix timestamp in seconds!

## Transfer Ownership

`client.transferOwnership(args)` -- transfer the ownership of specified
locker to someone else.

* `args`:

```js
{
    // Locker account as returned from `getLockers`.
    locker,
    // `anchor.web3.PublicKey` of a new owner.
    newOwner,
}
```

## Increment Lock

`client.incrementLock(args)` -- add more tokens to locker. It's cheaper than
creation new locker.

* `args`:

```js
{
    // Locker account as returned from `getLockers`.
    locker,
    // Amount to lock as `anchor.BN`.
    amount,
    // `anchor.web3.PublicKey` of funding wallet owner.
    // It should sign the transaction, so it's better to use
    // `provider.wallet.publicKey`.
    fundingWalletAuthority,
    // `anchor.web.PublicKey` of SPL token account from which
    // the locker will be funded. The amount of tokens you
    // specified earlier will be transferred from this account
    // to some program-controlled vault.
    fundingWallet,
}
```


## Withdraw Funds

`client.withdrawFunds(args)` -- withdraw the funds from locker.
Returns resulting `targetWallet` (associated or original).

If there's `startEmission` specified, you can withdraw funds linearly
in the period from `startEmission` to `unlockDate` -- it's called linear
emission.

If there's no linear emission available, you should wait till the `unlockDate`.

There's **NO** linear emission for LP lockers.

* `args`:

```js
{
    // Amount to withdraw as `anchor.BN`.
    amount,
    // Locker account as returned from `getLockers`.
    locker,
    // `boolean`. Flag specified if the transaction should use associated token
    // account if it's exists (or create the one if it's not).
    // If set to `true`, `targetWallet` should be ordinary account public key
    // (for example, `provider.wallet.publicKey`).
    // If set to `false`, `targetWallet` should be an SPL token account. In this
    // case no associated token account will be created. It's useful if the user
    // already has some token account.
    createAssociated,
    // `anchor.web.PublicKey` of a wallet to transfer tokens to.
    // If `createAssociated` is set to `true`, then associated SPL token account
    // will be created for this ordinary Solana account.
    // If `createAssociated` set to `false`, it should be SPL token account.
    targetWallet,
}
```

## Split the Locker

`client.splitLocker(args)` -- splits the locker into two parts.

* `args`:

```js
{
    // Amount to deposit in a new locker as `anchor.BN`.
    amount,
    // Locker account as returned from `getLockers`.
    locker,
    // `anchor.web.PublicKey` of a new owner.
    newOwner,
}
```

## Close locker (for tests only!)

`client.closeLocker(args)`

* `args`:

```js
{
    // Locker account as returned from `getLockers`.
    locker,
    // `anchor.web.PublicKey` of a wallet to transfer tokens to.
    // Should be an SPL token account!
    targetWallet,
}
```

## Check if token is already whitelisted

`client.isMintWhitelisted(mint)` -- if the mint is whitelisted, it's
possible to create new lockers without any fees.

* `mint` -- SPL token `anchor.web.PublicKey`

Returns simple boolean.

## Find vault authority address

`client.vaultAuthorityAddress(locker)` -- returns vault authority for
given locker.

* `locker` -- as returned from `getLockers`.

## Check if token is accepted

`client.isTokenAccepted(mint)` -- returns a boolean that indicates
it the token is possible to lock. Always returns `true` for token locker.

* `mint` -- SPL token address.
