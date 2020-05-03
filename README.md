# btfm

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

Create the data directory and database:

```
export BTFM_DATA_DIR=/some/dir
export DATABASE_URL=$BTFM_DATA_DIR/btfm.sqlite3
cargo install diesel_cli
diesel setup
```

To run, the following environment variables are required:

  * `BTFM_DATA_DIR`: Path to the data directory where btfm should store clips
    and where the database is. A special "hello" audio file must be in the
    root of this directory and is played on joins to the channel
  * `DEEPSPEECH_MODEL`: Path to the deepspeech model.
  * `DISCORD_TOKEN`: Your Discord API token.
  * `CHANNEL_ID`: The Discord Channel ID to join when someone else joins.
  * `GUILD_ID`: The Discord Guild ID being connected to.


## Development environment

If you are so inclined, there is a Dockerfile and some helper scripts in the ```devel/``` folder
that you may find to be handy for development. The scripts assume you have
[podman](https://podman.io/) installed. You can use ```build.sh``` to build a development container,
and you can use ```cargo.sh``` to run Rust's [cargo](https://doc.rust-lang.org/cargo/) tool inside
the container. You can probably guess what ```test.sh``` does, if you are somebody's kid and are
smart.
