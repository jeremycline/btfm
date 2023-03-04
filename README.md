# btfm

![CI](https://github.com/jeremycline/btfm/workflows/CI/badge.svg?branch=main)
![Security](https://github.com/jeremycline/btfm/workflows/Security%20audit/badge.svg?branch=main)

[I don't know how, But They Found Me (BTFM)](https://www.youtube.com/watch?v=hslfuqhtn7A).

btfm is a [Discord](https://discordapp.com) bot that listens on a voice channel for key phrases, and
plays audio clips into the channel in response.


## Setup

### Discord Registration

You'll need to register a bot with Discord. Go to [the Developer application
page](https://discord.com/developers/applications) and create an application.

### PostgreSQL

BTFM uses a PostgreSQL database to store audio clip metadata. Install PostgreSQL and create a database. For example:

```
sudo apt install postgresql postgresql-contrib
sudo systemctl restart postgresql.service
sudo -u postgres createuser btfm
sudo -u postgres createdb btfm
sudo -u postgres psql -c "ALTER USER btfm PASSWORD 'password';"
sudo -u postgres psql -c "ALTER DATABASE btfm OWNER to btfm;"
```

The `btfm-server` service will create the database schema when it connects. Any migrations required will also
be run automatically on updates.

### Configuration

Create a user for the bot:

```
$ sudo useradd --home-dir=/var/lib/btfm --create-home btfm
```

An example configuration file:

```
# The directory where audio clips and other data is stored.
# The server will create two directories in the data directory: "clips" contains
# the uploaded audio clips and "tts_cache" stores any text-to-speech audio it creates.
# The "tts_cache" directory can be safely removed if it grows too large.
data_directory = "/var/lib/btfm/"
# The database created in the prior step; substitute the password as necessary.
database_url = 'postgres://btfm:password@localhost/btfm'
# This is the Discord API token you got during the Discord registration step.
discord_token = 'your discord token here'
# The channel to join when someone enters; this is available by enabling "Developer Mode"
# in the Discord client in the advanced settings, then right-clicking a voice channel and
# copying the ID.
channel_id = 0
# The server ID to join; this is available by enabling "Developer Mode" in the Discord
# client in the advanced settings, then right-clicking a server and copying the ID.
guild_id = 0
# The optional channel to log events to; when a clip is matched the bot will post a message
# in this text channel.
log_channel_id = 0
# Adjust the frequency of playing clips; the odds of a clip being played is
# 1 - e^(-x/rate_adjuster) where "x" is the number of seconds since the last clip was played.
rate_adjuster = 100
# The bot will play a random clip at the interval provided (in seconds)
random_clip_interval = 900
# If set, this is the URL for a mimic3 HTTP API used to convert text-to-speech so the bot can
# talk back.
mimic_endpoint = "http://localhost:8888/api/"

[whisper]
# The path to the OpenAI Whisper model to use for transcription.
model = "/var/lib/btfm/whisper/base.en.pt"

[http_api]
# Where the HTTP API used for management listens.
url = "127.0.0.1:8080"
# The username required to authenticate with the management API
user = "admin"
# The password required to authenticate with the management API
password = "admin"

# To enable TLS for the HTTP API, set the following two keys. If
# TLS should not be used, ensure these keys don't exist.
#
# If set, tls_key must also be set and the HTTP API will use TLS
tls_certificate = "/var/lib/btfm/fullchain.pem"
# If set, tls_certificate must also be set and the HTTP API will use TLS
tls_key = "/var/lib/btfm/privkey.pem"
```

You can place this, for example, in ``/var/lib/btfm/btfm.toml``.

### Python Environment

The Whisper Python API is used to perform transcription, so you need to set up a Python
environment and install Whisper. The recommended approach is as follows (assuming you're using
Fedora Linux):

```
sudo dnf install python3-pip
sudo -u btfm bash -c \
    'python3 -m venv --upgrade-deps $HOME/.whisper && \
    $HOME/.whisper/bin/pip install openai-whisper'
```

Next, test the installation and download the model to use (replace the model as necessary and be
sure to adjust the configured model in btfm.toml to match):

```
sudo -u btfm bash -c '$HOME/.whisper/bin/whisper --model base.en --model_dir $HOME/whisper/ <an audio file>
```

### systemd

An example systemd unit to run BTFM:

```
[Unit]
Description=BTFM Discord bot
After=network.target

[Service]
Type=simple
User=btfm
Group=btfm
Environment="PATH=/var/lib/btfm/.whisper/bin:/usr/bin:/usr/local/bin/"
Environment="BTFM_CONFIG=/var/lib/btfm/btfm.toml"
Environment="RUST_LOG=warn,btfm=info"
ExecStart=/usr/local/bin/btfm-server run
Restart=always
RestartSec=60

[Install]
WantedBy=multi-user.target
```

### Building

If building from source, install make, autotools, libopus headers, gstreamer headers, libsodium headers, and the openssl headers.

### Usage

Add clips and phrases with the ``btfm clip`` sub-commands:

```
btfm clip add --phrase "they found me" "I don't know how, but they found me..." run-for-it-marty.mp3
```

See ``btfm clip --help`` for available sub-commands and options.

Start the bot with ``btfm-server run``. See the systemd unit above for details.

See ``btfm-server run --help`` for command line arguments and documentation. To
obtain the guild and channel ID, go to your Discord User Settings ->
Appearance, and enable Developer Mode. You can then right-click on the server
for and select "Copy ID" for the guild ID, and then right-click the voice
channel you want the bot to watch and "Copy ID" that as well.


## Development environment

If you are so inclined, there is a Dockerfile and some helper scripts in the ```devel/``` folder
that you may find to be handy for development. The scripts assume you have
[podman](https://podman.io/) installed. You can use ```build.sh``` to build a development container,
and you can use ```cargo.sh``` to run Rust's [cargo](https://doc.rust-lang.org/cargo/) tool inside
the container. You can probably guess what ```test.sh``` does, if you are somebody's kid and are
smart.
