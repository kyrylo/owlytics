# Owlytics

Small, self-hosted web analytics built with Rust and SQLite. One executable, one
database file, one owner, as many websites as you need.

See visitors, pageviews, traffic sources, popular pages, and locations. Cookieless
tracking, no stored raw IP addresses, and support for importing Plausible history.

**Early beta.** Keep your current analytics running while evaluating Owlytics.

## Install

Install Rust 1.96, then build:

```sh
git clone https://github.com/kyrylo/owlytics.git
cd owlytics
cargo build --release --locked
```

Choose an owner password of at least 12 characters and start Owlytics (Bash/Zsh):

```sh
printf 'Owner password: '
IFS= read -r -s owlytics_password
printf '\n'
export OWLYTICS_PASSWORD_HASH="$(printf '%s' "$owlytics_password" | target/release/owlytics hash-password)"
unset owlytics_password
target/release/owlytics
```

Open [localhost:3000](http://localhost:3000) and sign in with that password. Select
**Add website**, then paste its tracker snippet into your website's `<head>`.
There is no default password or username. Change your password in **Settings**.

Data is stored in `data/owlytics.sqlite`. Keep this directory between restarts.
Until you change the password in Settings or with `reset-password`, supply a valid
`OWLYTICS_PASSWORD_HASH` each time you start the server; save it in your service
manager's environment configuration. In this mode, changing the environment hash
changes the password and invalidates existing sessions.

Settings and `reset-password` save the new credential in SQLite. That credential
takes precedence on every subsequent start: `OWLYTICS_PASSWORD_HASH` is optional
and ignored, even if it is invalid. Changing or removing the environment hash
cannot override a stored password. If SQLite has no stored credential, a missing
or invalid environment hash prevents startup with an error.

See [.env.example](.env.example) for the available settings. Owlytics reads
environment variables, not `.env` files.

### Docker

With `OWLYTICS_PASSWORD_HASH` exported above:

```sh
docker build -t owlytics:local .
docker run -d --name owlytics --restart unless-stopped \
  -p 127.0.0.1:3000:3000 -v owlytics_data:/data \
  -e OWLYTICS_PASSWORD_HASH -e OWLYTICS_URL=http://localhost:3000 \
  owlytics:local
```

Stop the native server first if it is using port 3000. The Docker volume keeps
your data. For public access, use an HTTPS reverse proxy, set `OWLYTICS_URL` to
your public origin, and set `OWLYTICS_TRUSTED_PROXIES` to the proxy's IP or CIDR.
Keep port 3000 private.

### Reset a forgotten password

`reset-password` reads the new password from stdin, saves it in the existing
SQLite database, clears the login rate limit, and signs out all sessions. It
works without `OWLYTICS_PASSWORD_HASH`, including when that variable is invalid.
Choose a password of at least 12 characters (at most 1024 bytes). The commands
below use a hidden prompt in Bash/Zsh and a shell builtin to pipe the password;
do not put the password in command arguments. The subshell disables tracing and
discards the password variable when it exits.

For a native installation, run from the repository directory as the user who owns
the database. Adjust the binary and database paths to match your installation:

```sh
(
  set +x
  unset owlytics_password
  printf 'New owner password: ' >&2
  IFS= read -r -s owlytics_password || exit 1
  printf '\n' >&2
  builtin printf '%s' "$owlytics_password" |
    OWLYTICS_DATABASE=data/owlytics.sqlite target/release/owlytics reset-password
)
```

For the Docker installation above, use the same image and data volume as your
server. This disposable container works whether the server is running or stopped.
Keep `-i` to pass stdin; do not add `-t`:

```sh
(
  set +x
  unset owlytics_password
  printf 'New owner password: ' >&2
  IFS= read -r -s owlytics_password || exit 1
  printf '\n' >&2
  builtin printf '%s' "$owlytics_password" |
    docker run --rm -i -v owlytics_data:/data owlytics:local reset-password
)
```

The command requires an existing database. If you use a different volume, bind
mount, or `OWLYTICS_DATABASE` path, use that same storage and path for the reset.
After success, sign in with the new password; a running server needs no restart.
If the server was stopped, start it normally. You can remove the environment hash
from its configuration once the credential is stored in SQLite.

### Deploy with Kamal

The included [Kamal configuration](config/deploy.yml) builds the image, provisions
HTTPS, and mounts a persistent data volume. Install Kamal 2.12, point your hostname
at your server, and export the deployment variables in [.env.example](.env.example).
Set `OWLYTICS_TRUSTED_PROXIES` to the server's actual Kamal Docker network CIDR
(`docker network inspect kamal`; create the network first on a new host). Export
`KAMAL_REGISTRY_PASSWORD` with a token that can push to your image registry, and
use the password hash generated above.

```sh
cp .kamal/secrets.example .kamal/secrets
kamal setup
```

Open `https://YOUR_HOSTNAME` and sign in. For later updates, run `kamal deploy`.

## License

[MIT](LICENSE). Bundled [icons](docs/licenses/icons.md) and
[geographic data](docs/licenses/geography.md) have their own licenses.
