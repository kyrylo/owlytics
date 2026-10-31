# Owlytics

Small, self-hosted web analytics. One Rust executable, one SQLite database,
as many websites as you need.

See how people find your site and what they read. Visitors, pageviews, traffic
sources, popular pages, and locations are all there. You can bring your history
from Plausible, too.

No cookies. No stored raw IP addresses. Owlytics respects Do Not Track and Global
Privacy Control by default. [More about collection](docs/privacy.md).

**It's an early beta.** Keep your current analytics running while you try it out.

## Try it

Install Rust 1.96, then build:

```sh
git clone https://github.com/kyrylo/owlytics.git
cd owlytics
cargo build --release --locked
```

Pick a password with at least 12 characters and start Owlytics (Bash/Zsh):

```sh
printf 'Owner password: '
IFS= read -r -s owlytics_password
printf '\n'
export OWLYTICS_PASSWORD_HASH="$(printf '%s' "$owlytics_password" | target/release/owlytics hash-password)"
unset owlytics_password
target/release/owlytics
```

Open [localhost:3000](http://localhost:3000) and sign in with that password. No
username needed. Click **Add website** and paste the tracker snippet into your
site's `<head>`. You're ready to collect stats.

You can also [run it with Docker](docs/installation.md#docker).

## Keep it running

Your data lives in `data/owlytics.sqlite`. Keep that directory between restarts.
Save `OWLYTICS_PASSWORD_HASH` in your service manager's environment, or change
your password in **Settings** to save it in the database.

All settings are in [.env.example](.env.example). Export them yourself or use your
service manager; Owlytics doesn't load `.env` files.

- [Deploy with Kamal](docs/installation.md#deploy-with-kamal)
- [Change or reset your password](docs/administration.md)

## License

[MIT](LICENSE). Bundled [icons](docs/licenses/icons.md) and
[geographic data](docs/licenses/geography.md) have their own licenses.
