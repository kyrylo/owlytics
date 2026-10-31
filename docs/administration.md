# Passwords

[Back to the README](../README.md)

There's one owner account and no username. To change your password, open
**Settings**. If you've forgotten it, use `reset-password` below.

## How passwords are stored

Until you change the password in Settings or with `reset-password`, supply a valid
`OWLYTICS_PASSWORD_HASH` each time you start the server. Save it in your service
manager's environment configuration. Changing this hash changes the password and
signs out existing sessions.

Settings and `reset-password` save the new credential in SQLite. From then on,
`OWLYTICS_PASSWORD_HASH` is optional and ignored, even if it's invalid. Changing
or removing it can't override the stored password.

If SQLite has no stored credential, a missing or invalid environment hash prevents
startup with an error. Keep your database between restarts: `data/owlytics.sqlite`
for the native setup, or the `owlytics_data` volume for Docker.

## Reset a forgotten password

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

For the [Docker installation](installation.md#docker), use the same image and data
volume as your server. This disposable container works whether the server is
running or stopped. Keep `-i` to pass stdin; do not add `-t`:

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
