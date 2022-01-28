# Locker

## Deploy

Install Anchor -- described [here](https://project-serum.github.io/anchor/getting-started/installation.html#install-anchor).

```bash
cd lp-locker
# build all the required programs
anchor build
# deploy everything to devnet
# this command will deploy program ids
anchor deploy --provider.cluster devnet
# there's caveat -- we need to deploy lp locker separately
# using command that anchor is calling underneath
# but before that we need to generate program keypair
solana-keygen new -o lp-locker-keypair.json
solana program deploy \
# this is analogous to `cluster` parameter
--url devnet \
# it's yours default wallet
--keypair ~/.config/solana/id.json \
# keypair generated above
--program-id lp-locker-keypair.json \
# it's the path to program binary
target/deploy/locker.so
# now we need to initialize country list
# this command outputs country list pubkey
cargo run -p admin-cli -- \
--cluster devnet \
--program-id <country list program id from anchor deploy cmd> \
country-list init \
--countries './Country List.csv'
# after that we have to initialize config for token locker
cargo run -p admin-cli -- \
--cluster devnet \
--program-id <program id for token locker> \
locker init-config \
--country-list <country list pubkey from above> \
--fee-wallet <some fee wallet pubkey> \
--preset token-locker
# and for lp locker
cargo run -p admin-cli -- \
--cluster devnet \
--program-id <program id for lp locker> \
locker init-config \
--country-list <country list pubkey from above> \
--fee-wallet <some fee wallet pubkey> \
--preset lp-locker
# since only admins can add tokens for lp locker
# we need to add some token now
cargo run -p admin-cli -- \
--cluster devnet \
--program-id <program id for lp locker> \
locker add-token \
--mint <mint pubkey here>
```
