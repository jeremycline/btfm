# btfm

![CI](https://github.com/jeremycline/btfm/workflows/CI/badge.svg?branch=main)
![Security](https://github.com/jeremycline/btfm/workflows/Security%20audit/badge.svg?branch=main)

[I don't know how, But They Found Me (BTFM)](https://www.youtube.com/watch?v=hslfuqhtn7A).

btfm is a [Discord](https://discordapp.com) bot that listens on a voice channel for key phrases, and
plays audio clips into the channel in response.


## Setup

Download the [deepspeech native_client build for your
platform](https://github.com/mozilla/DeepSpeech/releases/tag/v0.9.3), along
with the
[acoustic
model](https://github.com/mozilla/DeepSpeech/releases/download/v0.9.3/deepspeech-0.9.3-models.pbmm)
and [external
scorer](https://github.com/mozilla/DeepSpeech/releases/download/v0.9.3/deepspeech-0.9.3-models.scorer).

If you're not sure which deepspeech build you need, you likely want [the x86_64
CPU
build](https://github.com/mozilla/DeepSpeech/releases/download/v0.9.3/native_client.amd64.cpu.linux.tar.xz)

If you don't want to build BTFM from source, there's an [x86_64 build for
Linux](https://github.com/jeremycline/btfm/releases) for each release that you
can download. For example:

```
wget https://github.com/jeremycline/btfm/releases/download/v0.13.0/btfm-x86_64-unknown-linux-gnu
mv btfm-x86_64-unknown-linux-gnu /usr/local/bin/btfm
```

### DeepSpeech

Extract the deepspeech shared library and set up your paths so that it can be found; for example, on Fedora:

```
wget "https://github.com/mozilla/DeepSpeech/releases/download/v0.9.3/native_client.amd64.cpu.linux.tar.xz"
mkdir deepspeech && tar -xvf native_client.amd64.cpu.linux.tar.xz -C deepspeech
sudo cp deepspeech/libdeepspeech.so /usr/local/lib64/
echo "/usr/local/lib64" | sudo tee -a /etc/ld.so.conf.d/local.conf
sudo ldconfig
```

### PostgreSQL

BTFM uses a PostgreSQL database to store audio clip metadata. Install PostgreSQL and create a database. For example:

```
sudo apt install postgresql postgresql-contrib
sudo systemctl restart postgresql.service
sudo -u postgres createuser btfm
sudo -u postgres createdb btfm
sudo -u postgres psql -c "ALTER USER btfm PASSWORD 'password';"
sudo -u postgres psql -c "ALTER DATABASE btfm OWNER to btfm;"
export DATABASE_URL=postgres://btfm:password@localhost/btfm

# Create the initial database tables
cargo install sqlx-cli
cargo sqlx database setup
```

### Configuration

Create the data directory where the audio clips are stored. For example:

```
mkdir /var/lib/btfm/
```

An example configuration file:

```
# The directory where audio clips and other data is stored
data_directory = '/var/lib/btfm/'
database_url = 'postgres://btfm:password@localhost/btfm'
discord_token = 'your discord token here'
# The channel to join when someone enters
channel_id = 0
# The guild ID to join
guild_id = 0
# The optional channel to log transcriptions to
log_channel_id = 0
# Adjust the frequency of playing clips
rate_adjuster = 100

[deepspeech]
model = '/var/lib/btfm/deepspeech.pbmm'
scorer = '/var/lib/btfm/deepspeech.scorer'
```

You can place this, for example, in ``/var/lib/btfm/btfm.toml``.

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
Environment="BTFM_CONFIG=/var/lib/btfm/btfm.toml"
ExecStart=/usr/local/bin/btfm run -v
Restart=always
RestartSec=60

[Install]
WantedBy=multi-user.target
```

### Building

If building from source, install make, autotools, libopus headers, libsqlite headers, libsodium headers, and the openssl headers.

### Usage

Add clips and phrases with the ``btfm clip`` sub-commands:

```
BTFM_CONFIG=/var/lib/btfm/btfm.toml btfm add "they found me" "I don't know how, but they found me..." run-for-it-marty.mp3
```

See ``btfm clip --help`` for available sub-commands and options.

Start the bot with ``btfm run``. See the systemd unit above for details.```

See ``btfm run --help`` for command line arguments and documentation. To
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
