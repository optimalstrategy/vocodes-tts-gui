# vo.codes TTS GUI
This is a simple program for downloading .wav TTS files from https://vo.codes.

This application is based on [Discord-TTS](https://github.com/MysteryPancake/Discord-TTS) by MysteryPancake.

## Usage
1. Download a binary from the [release page](https://github.com/optimalstrategy/vocodes-tts-gui/releases/tag/v0.1.0).
2. Choose a voice and enter your prompt.
3. Optionally, assign a custom filename.
4. Click "Download". The resulting wav file will be saved to the current directory.

## Building from source
1. Clone the repo
2. Run `cargo build --release`
3. Find the binary in `target/release`
4. (Optional) Set the `RUST_LOG` env variable to a value of choice, e.g. `DEBUG`
5. Run the binary: `./tts-gui`
