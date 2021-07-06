use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::{egui, epi};

mod voices;

fn generate_filename(voice: &str, content: &str) -> String {
    let prefix = content
        .split_whitespace()
        .take(4)
        .map(|s| s.to_ascii_lowercase())
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

#[derive(Debug)]
struct TtsPrompt {
    voice: &'static str,
    prompt: String,
    filename: String,
}
type TtsResult = Result<(), Error>;

struct TtsSubmitter {
    prompt_tx: Sender<TtsPrompt>,
    result_rx: Receiver<TtsResult>,
}

struct TtsReceiver {
    prompt_rx: Receiver<TtsPrompt>,
    result_tx: Sender<TtsResult>,
}

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
        let client = reqwest::blocking::Client::new();
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

struct Error {
    title: String,
    message: String,
    should_exit: bool,
    acknowledged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Idle,
    Processing,
    Success,
}

pub struct VoCodesTts {
    submitter: TtsSubmitter,
    prompt: String,
    voice: &'static str,
    filename: String,
    error: Option<Error>,
    status: Status,
}

impl VoCodesTts {
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
}

impl Default for VoCodesTts {
    fn default() -> Self {
        Self {
            submitter: spawn_downloader_thread(),
            error: None,
            voice: "sonic",
            prompt: "A test message".to_owned(),
            filename: generate_filename("sonic", "A test message"),
            status: Status::Idle,
        }
    }
}

impl epi::App for VoCodesTts {
    fn name(&self) -> &str {
        "egui template"
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
            ui.heading("Vo.Codes TTS Downloader");

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
                *filename = generate_filename(voice, &prompt);
            }

            ui.label("Enter the filename: ");
            ui.text_edit_singleline(filename);

            if *status == Status::Processing {
                ui.output().cursor_icon = egui::CursorIcon::Progress;
            } else {
                ui.output().cursor_icon = egui::CursorIcon::Default;
            }

            ui.set_enabled(!(*status == Status::Processing));

            ui.horizontal(|ui| {
                if ui.button("Download").clicked() {
                    if let Err(e) = submitter.prompt_tx.send(TtsPrompt {
                        prompt: prompt.clone(),
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
                    *status = Status::Processing;
                }
                ui.add(
                    egui::Label::new(format!("(status: {:?})", status)).text_color(match *status {
                        Status::Idle => egui::Color32::WHITE,
                        Status::Processing => egui::Color32::YELLOW,
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
