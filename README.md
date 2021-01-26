# btfm

![CI](https://github.com/jeremycline/btfm/workflows/CI/badge.svg?branch=main)
![Security](https://github.com/jeremycline/btfm/workflows/Security%20audit/badge.svg?branch=main)

[I don't know how, But They Found Me (BTFM)](https://www.youtube.com/watch?v=hslfuqhtn7A).

btfm is a [Discord](https://discordapp.com) bot that listens on a voice channel for key phrases, and
plays audio clips into the channel in response.


## Setup

Download the [deepspeech native_client build for your
platform](https://github.com/mozilla/DeepSpeech/releases/tag/v0.7.0), along
with the
[acoustic model](https://github.com/mozilla/DeepSpeech/releases/download/v0.7.0/deepspeech-0.7.0-models.pbmm).

Set up your paths so that deepspeech can be found by the compiler (or drop it into /usr/local/lib/ and run ldconfig).

Install make, autotools, libopus headers, libsqlite headers, libsodium headers, and the openssl headers.

Create the data directory where the audio clips and database are stored. For example:

```
mkdir /var/lib/btfm/
```

Add clips and phrases with the ``btfm clip`` sub-commands:

```
btfm --btfm-data-dir /var/lib/btfm/ add "they found me" "I don't know how, but they found me..." run-for-it-marty.mp3
```

See ``btfm clip --help`` for available sub-commands and options.

Start the bot with ``btfm run``. Parameters are accepted via CLI arguments or
environment variables. For example, a systemd unit file to run the service
under the "btfm" user (which should be able to read /var/lib/btfm/):
```
[Unit]
Description=BTFM Discord bot
After=network.target

[Service]
Type=simple
User=btfm
Group=btfm
Environment="BTFM_DATA_DIR=/var/lib/btfm/"
Environment="DEEPSPEECH_MODEL=/var/lib/btfm/deepspeech.pbmm"
Environment="DEEPSPEECH_SCORER=/var/lib/btfm/deepspeech.scorer"
Environment="DISCORD_TOKEN=<your-discord-api-token>"
Environment="CHANNEL_ID=<the-voice-channel-id>"
Environment="GUILD_ID=<the-guild-id>"
ExecStart=/usr/local/bin/btfm run
Restart=always
RestartSec=60

[Install]
WantedBy=multi-user.target

[Install]
WantedBy=multi-user.target
```

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
