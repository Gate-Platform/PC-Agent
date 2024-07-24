use whisper_rs::{ WhisperContext, WhisperContextParameters, FullParams, SamplingStrategy };
use cpal::traits::{ DeviceTrait, HostTrait, StreamTrait };
use cpal::{ SampleFormat, SupportedStreamConfig };
use std::thread;
use std::sync::Arc;
use std::sync::Mutex as SyncMutex;
use tokio::sync::Mutex;
use anyhow::Result;
use tokio::runtime::Runtime;

pub struct AudioManager {
    shared_context: Arc<SharedWhisperContext>,
}

impl AudioManager {
    pub fn new(model_path: &str, max_chars: usize) -> Result<Self> {
        let shared_context = Arc::new(SharedWhisperContext::new(model_path, max_chars)?);
        Ok(Self { shared_context })
    }

    pub fn start_audio_capture(&self) -> Result<()> {
        let shared_context = Arc::clone(&self.shared_context);
        thread::spawn(move || {
            if let Err(e) = run_audio_capture_and_transcription(shared_context) {
                eprintln!("Audio capture and transcription error: {:?}", e);
            }
        });
        Ok(())
    }
    pub async fn reset_transcript(&self) {
        self.shared_context.reset_transcript().await;
    }
    pub fn set_enabled(&self, set: bool) {
        println!("Attempting to set enabled to: {}", set);
        match self.shared_context.enabled.lock() {
            Ok(mut guard) => {
                println!("Lock acquired. Current value: {}", *guard);
                *guard = set;
                println!("Value updated. New value: {}", *guard);
            }
            Err(_) => println!("Failed to acquire lock"),
        }
    }

    pub async fn get_full_transcription(&self) -> String {
        self.shared_context.full_transcription.lock().await.clone()
    }
}

// pub fn start_audio_context_capture() -> Result<()> {
//     thread::spawn(|| {
//         if let Err(e) = run_audio_capture_and_transcription() {
//             eprintln!("Audio capture and transcription error: {:?}", e);
//         }
//     });
//     Ok(())
// }

fn run_audio_capture_and_transcription(shared_context: Arc<SharedWhisperContext>) -> Result<()> {
    let runtime = Runtime::new()?;
    runtime.block_on(async { capture_and_transcribe(shared_context).await })
}

struct SharedWhisperContext {
    context: Arc<Mutex<WhisperContext>>,
    full_transcription: Arc<Mutex<String>>,
    enabled: Arc<SyncMutex<bool>>,
    max_chars: usize,
}
struct AudioBuffer {
    samples: Vec<f32>,
}

impl SharedWhisperContext {
    fn new(model_path: &str, max_chars: usize) -> Result<Self> {
        let context = WhisperContext::new_with_params(
            model_path,
            WhisperContextParameters::default()
        )?;
        Ok(Self {
            context: Arc::new(Mutex::new(context)),
            enabled: Arc::new(SyncMutex::new(true)),
            full_transcription: Arc::new(Mutex::new(String::new())),
            max_chars,
        })
    }
    pub async fn reset_transcript(&self) {
        let mut full_transcription_guard = self.full_transcription.lock().await;

        full_transcription_guard.clear();
    }
    pub async fn transcribe(
        &self,
        audio_data: Vec<f32>,
        config_for_stream: &SupportedStreamConfig
    ) -> Result<String> {
        let audio_data_16khz = channels_to_mono(
            resample(&audio_data, config_for_stream.sample_rate().0, 16000),
            config_for_stream.channels().into()
        );
        let context = self.context.lock().await;

        let mut state = context.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        params.set_n_threads(4);
        params.set_translate(true);
        params.set_language(Some("auto"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state.full(params, &audio_data_16khz)?;

        let num_segments = state.full_n_segments()?;
        let mut result = String::new();
        for i in 0..num_segments {
            let segment = state.full_get_segment_text(i)?;
            if segment.trim() != "[BLANK_AUDIO]" && !segment.trim().is_empty() {
                result.push_str(&segment);
                result.push('\n');
            }
        }

        if !result.trim().is_empty() {
            // Update the full transcription
            let mut full_transcription = self.full_transcription.lock().await;
            full_transcription.push_str(&result);

            // Apply the character limit
            if full_transcription.len() > self.max_chars {
                *full_transcription = full_transcription
                    .chars()
                    .skip(full_transcription.len() - self.max_chars)
                    .collect();
            }
        }

        Ok(result)
    }
}
async fn transcribe_audio(
    shared_context: &SharedWhisperContext,
    audio_data: Vec<f32>,
    config_for_stream: &SupportedStreamConfig
) -> Result<()> {
    shared_context.transcribe(audio_data, config_for_stream).await?;
    // println!("Transcription: {}", transcription);
    Ok(())
}

async fn capture_and_transcribe(shared_context: Arc<SharedWhisperContext>) -> Result<()> {
    // let shared_context = Arc::new(SharedWhisperContext::new("./src/assets/ggml-tiny-q5_1.bin")?);

    let host = cpal::default_host();
    let device = host.default_output_device().expect("Failed to get default output device");

    let config = device.default_output_config()?;
    let config_clone = config.clone(); // Clone the config before moving it

    println!("Default input config: {:?}", config);

    let audio_buffer = Arc::new(SyncMutex::new(AudioBuffer { samples: Vec::new() }));
    let audio_buffer_clone = Arc::clone(&audio_buffer);

    let stream = match config.sample_format() {
        SampleFormat::F32 =>
            device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mut buffer = audio_buffer_clone.lock().unwrap();
                        buffer.samples.extend_from_slice(data);
                    },
                    |err| eprintln!("An error occurred on the input audio stream: {}", err),
                    None
                )
                .unwrap(),
        SampleFormat::I16 =>
            device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let mut buffer = audio_buffer_clone.lock().unwrap();
                        buffer.samples.extend(data.iter().map(|&s| (s as f32) / 32768.0));
                    },
                    |err| eprintln!("An error occurred on the input audio stream: {}", err),
                    None
                )
                .unwrap(),
        SampleFormat::U16 =>
            device
                .build_input_stream(
                    &config.into(),
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let mut buffer = audio_buffer_clone.lock().unwrap();
                        buffer.samples.extend(data.iter().map(|&s| (s as f32) / 65535.0 - 0.5));
                    },
                    |err| eprintln!("An error occurred on the input audio stream: {}", err),
                    None
                )
                .unwrap(),
        _ => todo!(),
    };
    stream.play()?;

    // loop {
    //     stream.play()?;

    //     std::thread::sleep(std::time::Duration::from_secs(10));

    //     stream.pause()?;

    //     let audio_data = {
    //         let buffer = audio_buffer.lock().unwrap();
    //         buffer.samples.clone()
    //     };
    //     audio_buffer.lock().unwrap().samples.clear();
    //     transcribe_audio(&shared_context, audio_data, &config_clone);
    // }
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        let is_enabled = {
            let enabled = shared_context.enabled.lock().unwrap();
            println!("Current enabled status: {}", *enabled);
            *enabled
        };
        if is_enabled {
            println!("transcribing");
        } else {
            println!("not transcribing");
            continue;
        }

        let audio_data = {
            let mut buffer = audio_buffer.lock().unwrap();
            std::mem::take(&mut buffer.samples)
        };

        if !audio_data.is_empty() {
            let shared_context_clone = Arc::clone(&shared_context);
            let config_clone = config_clone.clone();
            8;
            tokio::spawn(async move {
                if
                    let Err(e) = transcribe_audio(
                        &shared_context_clone,
                        audio_data,
                        &config_clone
                    ).await
                {
                    eprintln!("Transcription error: {:?}", e);
                }
            });
        }
    }
}

fn channels_to_mono(audio_data: Vec<f32>, num_channels: usize) -> Vec<f32> {
    assert!(
        audio_data.len() % num_channels == 0,
        "Audio data length must be a multiple of the number of channels"
    );

    let mut mono_data = Vec::with_capacity(audio_data.len() / num_channels);

    for i in 0..audio_data.len() / num_channels {
        let sum: f32 = audio_data[i * num_channels..(i + 1) * num_channels].iter().sum();
        mono_data.push(sum / (num_channels as f32)); // Average the samples across channels
    }

    mono_data
}
fn resample(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    let ratio = (output_rate as f32) / (input_rate as f32);
    let output_length = ((input.len() as f32) * ratio) as usize;
    let mut output = vec![0.0; output_length];

    for i in 0..output_length {
        let input_index = (i as f32) / ratio;
        let input_index_floor = input_index.floor() as usize;
        let input_index_ceil = input_index.ceil() as usize;

        if input_index_ceil >= input.len() {
            output[i] = input[input.len() - 1];
        } else {
            let t = input_index - (input_index_floor as f32);
            output[i] = input[input_index_floor] * (1.0 - t) + input[input_index_ceil] * t;
        }
    }

    output
}
