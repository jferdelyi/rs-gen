use std::collections::HashSet;
use std::time::Duration;

use eframe::{egui, Frame};
use egui::Context;

use reqwest::blocking::Client;
use reqwest::Result;

/// Seed selection mode on the UI side.
/// This is intentionally UI-specific and serialized manually
/// to avoid invalid combinations.
#[derive(Debug, PartialEq)]
enum SeedMode {
    None,
    Custom,
    Random,
}

/// REST context holding a reusable blocking HTTP client.
struct RESTContext {
    client: Client,
}

impl RESTContext {
    /// Creates a new REST context with a timeout.
    fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::new(5, 0))
            .build()?;
        Ok(Self { client })
    }

    /// Sends a GET request to `/v1/generate` with query parameters.
    fn get_generated(&self, params: &[(String, String)]) -> Result<String> {
        let response = self.client
            .get("http://127.0.0.1:5000/v1/generate")
            .query(params)
            .send()?
            .error_for_status()?;

        // Remove quotes if the API returns a JSON string
        Ok(response.text()?.replace('"', ""))
    }

    /// Sends a GET request to `/v1/models` with query parameters.
    fn get_models(&self) -> Result<String> {
        let response = self.client
            .get("http://127.0.0.1:5000/v1/models")
            .send()?
            .error_for_status()?;

        // Remove quotes if the API returns a JSON string
        Ok(response.text()?.replace('"', ""))
    }

    /// Sends a GET request to `/v1/loaded_models` with query parameters.
    fn get_loaded_models(&self) -> Result<String> {
        let response = self.client
            .get("http://127.0.0.1:5000/v1/loaded_models")
            .send()?
            .error_for_status()?;

        // Remove quotes if the API returns a JSON string
        Ok(response.text()?.replace('"', ""))
    }

    /// Sends a PUT request to `/v1/load_models` with query parameters.
    fn put_load_models(&self, param: &str) -> Result<String> {
        let response = self.client
            .put("http://127.0.0.1:5000/v1/load_models")
            .query(&[("names", param)])
            .send()?
            .error_for_status()?;

        // Remove quotes if the API returns a JSON string
        Ok(response.text()?.replace('"', ""))
    }
}

/// Global UI state (MUST persist between frames in egui).
struct GeneratorUI {
    rest: RESTContext,
    last_word: Option<String>,
    selected_model: String,        // selected model from the dropdown
    selected_models: HashSet<String>, // 👈 IMPORTANT
    available_models: Vec<String>, // list of available models

    use_max_n: bool,
    max_n: usize,

    nb_try: usize,
    randomness: f32,
    reduce_random: bool,

    seed_mode: SeedMode,
    custom_seed: String,
    use_random_seed: bool,
    random_seed: usize
}

impl GeneratorUI {
    /// Initializes the UI with sane defaults.
    fn new() -> Result<Self> {
       let mut generator = Self {
            rest: RESTContext::new()?,
            last_word: None,

            selected_model: String::new(),
            selected_models: HashSet::new(),
            available_models: Vec::new(),

            use_max_n: false,
            max_n: 5,

            nb_try: 5,
            randomness: 0.1,
            reduce_random: false,

            seed_mode: SeedMode::None,
            custom_seed: String::new(),
            use_random_seed: false,
            random_seed: 2
        };
        generator.get_models();
        generator.get_loaded_models();
        Ok(generator)
    }

    /// Builds the query parameters for the API.
    ///
    /// IMPORTANT:
    /// - No unused parameters are sent
    /// - max_n = 0 is sent when checkbox is disabled
    fn build_query(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        // max_n logic
        params.push((
            "max_n".into(),
            if self.use_max_n {
                self.max_n.to_string()
            } else {
                "0".into()
            },
        ));

        params.push(("nb_try".into(), self.nb_try.to_string()));
        params.push(("randomness".into(), self.randomness.to_string()));
        params.push(("reduce_random".into(), self.reduce_random.to_string()));

        // Seed handling (mutually exclusive by design)
        match self.seed_mode {
            SeedMode::None => {}
            SeedMode::Custom if !self.custom_seed.is_empty() => {
                params.push(("seed".into(), format!("custom:{}", self.custom_seed)));
            }
            SeedMode::Random => {
                params.push((
                    "seed".into(),
                    if self.use_random_seed {
                        format!("random:{}", self.random_seed)
                    } else {
                        "random:0".to_string()
                    },
                ));
            }
            _ => {}
        }

        params
    }

    /// Performs the generation request.
    fn get_generated(&mut self) {
        let params = self.build_query();
        match self.rest.get_generated(&params) {
            Ok(word) => self.last_word = Some(word),
            Err(e) => self.last_word = Some(format!("Error: {e}")),
        }
    }

    /// Performs the get models request.
    fn get_models(&mut self) {
        match self.rest.get_models() {
            Ok(word) => self.available_models = word.split('\n').map(|s| s.trim().to_owned()).collect(),
            Err(e) => self.last_word = Some(format!("Error: {e}")),
        }
    }

    /// Performs the get models loaded request.
    fn get_loaded_models(&mut self) {
        match self.rest.get_loaded_models() {
            Ok(word) => self.selected_models = word.split('\n').map(|s| s.trim().to_owned()).collect(),
            Err(e) => self.last_word = Some(format!("Error: {e}")),
        }
    }

    /// Performs the load models request.
    fn put_load_models(&mut self, models: Vec<String>) {
        match self.rest.put_load_models(&*models.join(",")) {
            Ok(word) => self.last_word = Some(word),
            Err(e) => self.last_word = Some(format!("Error: {e}")),
        }
    }
}

impl eframe::App for GeneratorUI {
    /// UI update loop (called every frame).
    fn update(&mut self, ctx: &Context, _: &mut Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {

            egui::Grid::new("generator_grid")
                .num_columns(2)
                .spacing([20.0, 6.0])
                .striped(true)
                .show(ui, |ui| {

                    // max_n checkbox + value
                    ui.checkbox(&mut self.use_max_n, "Limit max n-gram length");
                    if self.use_max_n {
                        ui.add(
                            egui::DragValue::new(&mut self.max_n)
                                .range(2..=100)
                                .speed(1),
                        );
                    } else {
                        ui.label("Max n-gram length not limited");
                    }
                    ui.end_row();

                    // nb_try
                    ui.label("Number of tries");
                    ui.add(
                        egui::DragValue::new(&mut self.nb_try)
                            .range(1..=100)
                            .speed(1),
                    );
                    ui.end_row();

                    // randomness
                    ui.label("Randomness");
                    ui.add(
                        egui::DragValue::new(&mut self.randomness)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    );
                    ui.end_row();

                    // reduce_random
                    ui.label("Random event on reduce step");
                    ui.checkbox(&mut self.reduce_random, "");
                    ui.end_row();

                    ui.separator();
                    ui.end_row();

                    // seed mode
                    ui.label("Seed mode");
                    ui.vertical(|ui| {
                        ui.radio_value(&mut self.seed_mode, SeedMode::None, "None");
                        ui.radio_value(&mut self.seed_mode, SeedMode::Custom, "Custom");
                        ui.radio_value(&mut self.seed_mode, SeedMode::Random, "Random");
                    });
                    ui.end_row();

                    // custom seed input
                    if self.seed_mode == SeedMode::Custom {
                        ui.label("Custom seed");
                        ui.text_edit_singleline(&mut self.custom_seed);
                        ui.end_row();
                    }

                    // random seed input
                    if self.seed_mode == SeedMode::Random {
                        ui.checkbox(&mut self.use_random_seed, "Random but fixed n-gram length");
                        if self.use_random_seed {
                            ui.add(
                                egui::DragValue::new(&mut self.random_seed).range(2..=usize::MAX),
                            );
                        } else {
                            ui.label("Random seed (random prefix length)");
                        }
                        ui.end_row();
                    }

                    ui.separator();
                    ui.end_row();

                    // Generate button
                    if ui
                        .add_sized([200.0, 40.0], egui::Button::new("Generate"))
                        .clicked()
                    {
                        self.get_generated();
                    }

                    // Output
                    if let Some(word) = &self.last_word {
                        ui.label(word);
                    } else {
                        ui.label("Click Generate to start");
                    }

                    // Models
                   /* if ui.add_sized([200.0, 40.0], egui::Button::new("Get models from server")).clicked() {
                        self.get_models();
                    }*/

                    ui.end_row();


                });
            let mut selection_changed = false;

            for model in &self.available_models {
                let mut checked = self.selected_models.contains(model);

                if ui.checkbox(&mut checked, model).changed() {

                    if checked {
                        selection_changed = true;
                        self.selected_models.insert(model.clone());
                    } else {
                        if self.selected_models.len() > 1 {
                            selection_changed = true;
                            self.selected_models.remove(model);
                        }
                    }
                }
            }

            if selection_changed {
                let models = self.selected_models.iter().cloned().collect::<Vec<_>>();
                self.put_load_models(models);
            }




            // Drop-down for models
            /*
            ui.horizontal(|ui| {
                ui.label("Select model:");
                egui::ComboBox::from_label("")
                    .selected_text(&self.selected_model)
                    .show_ui(ui, |ui| {
                        for model in &self.available_models {
                            ui.selectable_value(&mut self.selected_model, model.clone(), model);
                        }
                    });
            });*/

            let previous_model = self.selected_model.clone();
            // Check if the user changed the selection
            if self.selected_model != previous_model {
                let models = self
                    .selected_model
                    .split(',')
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>();
                self.put_load_models(models);
            }
        });
    }
}

/// Application entry point.
fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([440.0, 380.0])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "rs-generator",
        options,
        Box::new(|_| Ok(Box::new(GeneratorUI::new()?))),
    )
}