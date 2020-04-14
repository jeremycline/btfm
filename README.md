# Setup

Download the [deepspeech build for your
platform](https://github.com/mozilla/DeepSpeech/releases/tag/v0.6.1), along
with the
[models](https://github.com/mozilla/DeepSpeech/releases/download/v0.6.1/deepspeech-0.6.1-models.tar.gz).

Set up your paths so that deepspeech can be found by the compiler (or drop it into /usr/local/lib/ and run ldconfig).

Install make, autotools, libopus headers, libsqlite headers, and the libsodium headers.

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
  * `DEEPSPEECH_MODEL_DIR`: Path to the deepspeech models.
  * `DISCORD_TOKEN`: Your Discord API token.
  * `CHANNEL_ID`: The Discord Channel ID to join when someone else joins.
  * `GUILD_ID`: The Discord Guild ID being connected to.
