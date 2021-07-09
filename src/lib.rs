use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::{egui, epi};

mod voices;

/// The number of seconds after which the connection will be dropped.
/// This is required since the TTS service sometimes hangs up forever for no apparent reason.
pub const TTS_TIMEOUT_SECONDS: u64 = 180;

/// A text prompt submitted by the user.
#[derive(Debug)]
struct TtsPrompt {
    /// The voice key to use
    voice: &'static str,
    /// The text to speak.
    prompt: String,
    /// The name of the resulting .wav file.
    filename: String,
}
type TtsResult = Result<(), Error>;

/// This struct is used by the GUI to submit prompts.
struct TtsSubmitter {
    /// The channel to submit the prompt.
    prompt_tx: Sender<TtsPrompt>,
    /// The channel to receive the result.
    result_rx: Receiver<TtsResult>,
}

/// This struct is used by the downloader thread to receive prompts and send back results.
struct TtsReceiver {
    /// The channel to receive prompts.
    prompt_rx: Receiver<TtsPrompt>,
    /// The channel to send back results.
    result_tx: Sender<TtsResult>,
}

/// Spawns a download thread and returns a struct holding the prompt sender and result receiver.
/// The thread will be stopped automatically when the sender is destroyed.
fn spawn_downloader_thread() -> TtsSubmitter {
    let (prompt_tx, prompt_rx) = unbounded();
    let (result_tx, result_rx) = unbounded();

    let submitter = TtsSubmitter {
        prompt_tx,
        result_rx,
    };
    let receiver = TtsReceiver {
        prompt_rx,
        result_tx,
    };

    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(TTS_TIMEOUT_SECONDS))
            .build()
            .unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        while let Ok(prompt) = receiver.prompt_rx.recv() {
            log::info!("Received a new prompt: {:#?}", prompt);
            log::info!("Making a request to the api...");

            let res = client
                .post("https://mumble.stream/speak")
                .headers(headers.clone())
                .body(format!(
                    "{{\"speaker\":\"{}\",\"text\":\"{}\"}}",
                    voices::TTS_VOICES[prompt.voice],
                    prompt.prompt
                ))
                .send();
            log::info!(
                "Received a response with the code: {:#?}",
                res.as_ref().map(|r| r.status())
            );
            log::debug!("Received a response: {:#?}", res);
            let result = match res {
                Ok(r) => {
                    if r.status().is_success() {
                        match r.bytes().map(|b| std::fs::write(&prompt.filename, b)) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(Error {
                                title: "Error: Failed to save the audio".to_string(),
                                message: e.to_string(),
                                should_exit: false,
                                acknowledged: false,
                            }),
                        }
                    } else {
                        Err(Error {
                            title: format!(
                                "Error: The server's response wasn't a success ({})",
                                r.status()
                            ),
                            message: match r
                                .text()
                                .unwrap_or_else(|_| "<Failed to get the error message>".into())
                            {
                                text if text.is_empty() => "(response was empty)".into(),
                                rest => rest,
                            },
                            should_exit: false,
                            acknowledged: false,
                        })
                    }
                }
                Err(e) => Err(Error {
                    title: "Error: Failed to generate audio".to_string(),
                    message: e.to_string(),
                    should_exit: false,
                    acknowledged: false,
                }),
            };
            let _ = receiver.result_tx.send(result);
        }
    });

    submitter
}

/// A struct containing the information about an error and two metadata fields.
struct Error {
    /// The title of the error window.
    title: String,
    /// The error message itself
    message: String,
    /// Whether the program should exit after the user acknowledges the error.
    should_exit: bool,
    /// Whether the error has been acknowledged by the user.
    acknowledged: bool,
}

/// The current state of the GUI.
#[derive(Clone, Copy, PartialEq)]
enum Status {
    /// Idle, the program is waiting for a prompt.
    Idle,
    /// Processing, the program is currently processing a prompt.
    Processing(Instant),
    /// Success, the program has finished processing a prompt.
    Success,
}

impl std::fmt::Debug for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Idle => write!(f, "Idle"),
            Status::Processing(instant) => write!(
                f,
                "Processing [{:0>3}s / {}s]",
                instant.elapsed().as_secs(),
                TTS_TIMEOUT_SECONDS
            ),
            Status::Success => write!(f, "Success"),
        }
    }
}

/// The state of the GUI.
pub struct VoCodesTts {
    /// The struct to submit prompts.
    submitter: TtsSubmitter,
    /// The current prompt.
    prompt: String,
    /// The currently selected voice.
    voice: &'static str,
    /// The filename to save the audio to.
    filename: String,
    /// The current error, if any.
    error: Option<Error>,
    /// The status of the GUI.
    status: Status,
}

impl VoCodesTts {
    /// Displays the given error, optionally exiting the program after the user acknowledges it.
    fn display_error(ctx: &egui::CtxRef, frame: &mut epi::Frame<'_>, error: &mut Error) {
        egui::Window::new(&error.title).show(ctx, |ui| {
            ui.add(
                egui::Label::new(&error.message)
                    .wrap(true)
                    .text_color(egui::Color32::RED),
            );
            if ui.button("OK").clicked() {
                error.acknowledged = true;
                if error.should_exit {
                    frame.quit()
                }
            }
        });
    }

    /// Generates af filename for the given voice and content pair, using the first 5 words of the message to start the filename.
    fn generate_filename(voice: &str, content: &str) -> String {
        let prefix = content
            .split_whitespace()
            .take(4)
            .map(|s| {
                s.to_ascii_lowercase()
                    .trim_matches(|c: char| !c.is_ascii_alphanumeric())
                    .to_owned()
            })
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("_");

        let date = chrono::Local::now();
        format!(
            "{}_{}_{}.wav",
            voice,
            prefix,
            date.format("%Y-%m-%d-%H%M%S")
        )
    }

    fn clean_prompt(prompt: &str) -> String {
        prompt
            .replace(|c: char| c.is_ascii_whitespace(), " ")
            .chars()
            .filter(|c| {
                c.is_ascii_digit()
                    || c.is_ascii_alphabetic()
                    || c.is_ascii_whitespace()
                    || [',', '.', '!', '?', '$', '\''].contains(c)
            })
            .collect::<String>()
    }
}

impl Default for VoCodesTts {
    fn default() -> Self {
        Self {
            submitter: spawn_downloader_thread(),
            error: None,
            voice: "sonic",
            prompt: "A test message".to_owned(),
            filename: Self::generate_filename("sonic", "A test message"),
            status: Status::Idle,
        }
    }
}

impl epi::App for VoCodesTts {
    fn name(&self) -> &str {
        "Vo.Codes TTS Downloader"
    }

    fn update(&mut self, ctx: &egui::CtxRef, frame: &mut epi::Frame<'_>) {
        let Self {
            error,
            voice,
            prompt,
            filename,
            submitter,
            status,
        } = self;

        match submitter.result_rx.try_recv() {
            Ok(Ok(_)) => {
                *status = Status::Success;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => (),
            Ok(Err(message)) => *error = Some(message),
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                *error = Some(Error {
                    title: "Error: The downloader thread has exited unexpectedly.".into(),
                    message:
                        "As the message says, the downloader thread has panicked for some reason. \
                    The application cannot continue functioning without it and must be shut down."
                            .into(),
                    should_exit: true,
                    acknowledged: false,
                });
            }
        }

        if let Some(error_value) = error {
            if error_value.acknowledged {
                *error = None;
                *status = Status::Idle;
            } else {
                Self::display_error(ctx, frame, error_value)
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_style(egui::Style::default());
            ui.heading(concat!(
                "Vo.Codes TTS Downloader (",
                env!("CARGO_PKG_VERSION"),
                ")"
            ));

            let prev_voice = *voice;
            egui::ComboBox::from_label("Choose a voice")
                .selected_text(format!("{:?}", voice))
                .show_ui(ui, |ui| {
                    for (name, _) in voices::TTS_VOICES.iter() {
                        ui.selectable_value(voice, name, &name);
                    }
                });

            ui.label("Enter your message: ");
            if ui.text_edit_multiline(prompt).changed() || *voice != prev_voice {
                let prompt = Self::clean_prompt(prompt);
                *filename = Self::generate_filename(voice, &prompt);
            }

            ui.label("Enter the filename: ");
            ui.text_edit_singleline(filename);

            if matches!(status, Status::Processing(_)) {
                ui.output().cursor_icon = egui::CursorIcon::Progress;
            } else {
                ui.output().cursor_icon = egui::CursorIcon::Default;
            }

            ui.set_enabled(!matches!(status, Status::Processing(_)) && !prompt.is_empty());

            ui.horizontal(|ui| {
                if ui.button("Download").clicked() {
                    if let Err(e) = submitter.prompt_tx.send(TtsPrompt {
                        prompt: Self::clean_prompt(prompt),
                        voice: *voice,
                        filename: filename.clone(),
                    }) {
                        *error = Some(Error {
                            title: "A critical error has occurred".to_string(),
                            message: e.to_string(),
                            should_exit: true,
                            acknowledged: false,
                        });
                    }
                    *status = Status::Processing(Instant::now());
                }
                ui.add(
                    egui::Label::new(format!("(status: {:?})", status)).text_color(match *status {
                        Status::Idle => egui::Color32::WHITE,
                        Status::Processing(_) => egui::Color32::YELLOW,
                        Status::Success => egui::Color32::GREEN,
                    }),
                );
            });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add(
                    egui::Hyperlink::new("https://github.com/optimalstrategy/vocodes-tts-gui/")
                        .text("source code"),
                );
            });

            egui::warn_if_debug_build(ui);
        });
    }
}
