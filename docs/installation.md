# Installation

[Back to the README](../README.md)

The [quick start](../README.md#try-it) builds and runs Owlytics with Rust.
You can also run it with Docker or deploy it with Kamal.

## Docker

Clone the repository and build the image:

```sh
git clone https://github.com/kyrylo/owlytics.git
cd owlytics
docker build -t owlytics:local .
```

Pick a password with at least 12 characters and generate its hash (Bash/Zsh).
You don't need Rust installed for this:

```sh
printf 'Owner password: '
IFS= read -r -s owlytics_password
printf '\n'
export OWLYTICS_PASSWORD_HASH="$(printf '%s' "$owlytics_password" | docker run --rm -i owlytics:local hash-password)"
unset owlytics_password
```

Stop any native Owlytics server first if it's using port 3000, then start the
container:

```sh
docker run -d --name owlytics --restart unless-stopped \
  -p 127.0.0.1:3000:3000 -v owlytics_data:/data \
  -e OWLYTICS_PASSWORD_HASH -e OWLYTICS_URL=http://localhost:3000 \
  owlytics:local
```

Open [localhost:3000](http://localhost:3000) and sign in with your password.
Click **Add website** and paste its tracker snippet into your site's `<head>`.

The `owlytics_data` volume keeps your data between containers. Reuse it when you
update. See [password administration](administration.md) for how the environment
hash and stored password work, including how to recover a forgotten password.

For public access, use an HTTPS reverse proxy, set `OWLYTICS_URL` to your public
origin, and set `OWLYTICS_TRUSTED_PROXIES` to the proxy's IP or CIDR. Keep port 3000
private. See [all settings](../.env.example); Owlytics doesn't load `.env` files.

## Deploy with Kamal

The included [Kamal configuration](../config/deploy.yml) builds the image, provisions
HTTPS, and mounts a persistent data volume. Install Kamal 2.12, point your hostname
at your server, and export the deployment variables in [.env.example](../.env.example).
Set `OWLYTICS_TRUSTED_PROXIES` to the server's actual Kamal Docker network CIDR
(`docker network inspect kamal`; create the network first on a new host). Export
`KAMAL_REGISTRY_PASSWORD` with a token that can push to your image registry, and
use a password hash generated with the Docker commands above or the
[quick start](../README.md#try-it).

```sh
cp .kamal/secrets.example .kamal/secrets
kamal setup
```

Open `https://YOUR_HOSTNAME` and sign in. For later updates, run `kamal deploy`.
