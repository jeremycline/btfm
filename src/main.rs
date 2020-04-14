use std::env;
use std::path::Path;
use std::sync::Arc;

use serenity::{
    client::{Client, Context},
    framework::{
        standard::{
            macros::{command, group},
            CommandResult,
        },
        StandardFramework,
    },
    model::{channel::Message, misc::Mentionable},
    voice,
};

use ::btfm::{Handler, Receiver, VoiceManager};

#[group]
#[prefix = "btfm"]
#[commands(join)]
struct General;

fn main() {
    env::var("DEEPSPEECH_MODEL_DIR")
        .expect("The DEEPSPEECH_MODEL_DIR enviroment variable must be defined.");
    let token =
        env::var("DISCORD_TOKEN").expect("The DISCORD_TOKEN environment variable must be defined.");
    let mut client = Client::new(&token, Handler).expect("Unable to create client");

    {
        let mut data = client.data.write();
        data.insert::<VoiceManager>(Arc::clone(&client.voice_manager));
    }

    client.with_framework(
        StandardFramework::new()
            .configure(|c| c.prefix("!"))
            .group(&GENERAL_GROUP),
    );
    let _ = client
        .start()
        .map_err(|why| println!("Client ended: {:?}", why));
}

#[command]
fn join(context: &mut Context, message: &Message) -> CommandResult {
    println!("Got a join command");

    // todo maybe just figure out what guild *I'm* in and join the channel based on that
    let guild = match message.guild(&context.cache) {
        Some(guild) => guild,
        None => {
            message
                .channel_id
                .say(&context.http, "Groups and DMs are not supported")
                .unwrap();
            return Ok(());
        }
    };

    let guild_id = guild.read().id;
    let channel_id = guild
        .read()
        .voice_states
        .get(&message.author.id)
        .and_then(|voice_state| voice_state.channel_id);
    let connect_to = match channel_id {
        Some(channel) => channel,
        None => {
            message
                .reply(&context, "You're not in a voice channel you dum-dum")
                .unwrap();
            return Ok(());
        }
    };

    let manager_lock = context
        .data
        .read()
        .get::<VoiceManager>()
        .cloned()
        .expect("Expected VoiceManager in ShareMap");
    let mut manager = manager_lock.lock();

    if let Some(handler) = manager.join(guild_id, connect_to) {
        handler.listen(Some(Box::new(Receiver::new(context.data.clone()))));

        // TODO make this a lot less bad
        let data_dir = env::var("BTFM_DATA_DIR")
            .expect("The BTFM_DATA_DIR environment variable must be defined.");
        let hello_agent = Path::new(&data_dir);
        let source = voice::ffmpeg(hello_agent.join(Path::new("hello.wav"))).unwrap();
        handler.play(source);
        message
            .channel_id
            .say(&context.http, &format!("Joined {}", connect_to.mention()))
            .unwrap();
    } else {
        message
            .channel_id
            .say(&context.http, "Unable to join channel")
            .unwrap();
    }

    Ok(())
}
