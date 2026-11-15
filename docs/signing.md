# Signing a setup file

Windows SmartScreen warns about unsigned programs from publishers it doesn't
know yet. A code-signing certificate makes the warning go away over time and
shows your name as the publisher when people download and run your setup file.

Bach setup files sign like any other program. Build first, then sign the
finished file:

```bash
bach build
```

```bash
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a "dist\My App-Setup-1.0.0.exe"
```

The signature is appended after Bach's payload and covers it, so any change
to the file after signing breaks the signature. Check it with:

```bash
signtool verify /pa "dist\My App-Setup-1.0.0.exe"
```

## Sign during the build (optional)

Signing is never required: without a `[signing]` section `bach build` writes
unsigned setup files, as before. If you'd rather not sign by hand, tell Bach
which program signs and with what. `bach build --sign` (or `BACH_SIGN=1`, which
suits CI) then signs every setup file it writes, the offline and the online
installer, before it writes the update feed, so the feed already carries the
signed file's SHA-256:

```toml
[signing]
program = "signtool"      # the default; any program that signs a file
arguments = '/fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a'
```

Bach runs `program` with `arguments` and the setup file added at the end
(write `{file}` where it belongs when it isn't last), without a shell, and
prints the command first. A signer that fails fails the build with its exit
code, so an unsigned file never goes out by accident.

Signing only ever runs when you ask for it with `--sign` or `BACH_SIGN=1`.
That is on purpose: a `bach.toml` that came from somewhere else can name any
program, and building it must not run that program behind your back. Keep secrets out of `bach.toml` by writing
`{env:NAME}`: Bach reads the variable when it builds and fails when it isn't
set. The command Bach prints shows `{env:NAME}` as written, never the value.

```toml
[signing]
# Azure Artifact Signing and other cloud signers work the same way.
program = "AzureSignTool"
arguments = 'sign --azure-key-vault-url {env:KV_URL} --azure-key-vault-certificate {env:KV_CERT} -tr http://timestamp.digicert.com'
```

```bash
bach build --sign                       # build and sign
bach sign dist/My-App-Setup-1.0.0.exe   # sign a file you built earlier
```

Without `--sign`, a build with `[signing]` says so and leaves the files
unsigned. Signing needs the signer on the machine that builds; `bach build` on macOS or
Linux can use a signer that runs there (for example `osslsigncode`). `bach
doctor` says whether `[signing]` is set.

Notes:

- Sign after `bach build`, never before: building writes into the file.
- The `uninstall.exe` an install leaves behind is unsigned. It's written on
  the user's computer from the signed setup file, without the signature,
  because a signature only covers the exact file it was made for.
- `bach inspect` reads signed setup files too.
- Signing with `[signing]` and signing by hand give the same result; use
  whichever fits your pipeline. After changing a signed file in any way,
  write the feed again with `bach feed`.
