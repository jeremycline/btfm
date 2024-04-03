use std::{fs::File, io::Write, path::PathBuf};



fn main() -> Result<(), Box<dyn std::error::Error>>{
    let out_dir = std::env::var("OUT_DIR")?;
    let client = reqwest::blocking::ClientBuilder::new().timeout(std::time::Duration::from_secs(30)).gzip(true).build().unwrap();
    let size = "small";

    for file in vec!["config.json", "tokenizer.json", "model.safetensors"].into_iter() {
        let target_path = PathBuf::from(format!("{out_dir}/{size}-{file}"));
        if !target_path.exists() {
            let url = format!("https://huggingface.co/openai/whisper-{size}/resolve/main/{file}?download=true");
            let response = client.get(url).send().unwrap();
            let bytes = response.bytes().unwrap();
            let mut file = File::create(&target_path).unwrap();
            file.write_all(&bytes).unwrap();
        }
    }

    println!("cargo::rustc-env=BTFM_MODEL_FILE={out_dir}/{size}-model.safetensors");
    println!("cargo::rustc-env=BTFM_MODEL_CONFIG_FILE={out_dir}/{size}-config.json");
    println!("cargo::rustc-env=BTFM_MODEL_TOKENIZER_FILE={out_dir}/{size}-tokenizer.json");

    Ok(())
}
