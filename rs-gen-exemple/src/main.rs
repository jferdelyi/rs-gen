use rs_gen_core::model::generator::Generator;
use rs_gen_core::model::prediction_input::StartSeed;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load all models from the "data" directory (.dat files)
    // Load automatically .bin is existing
    let mut app: Generator = Generator::new("./data")?;

    // Create a prediction input with default intensity values for all models
    let mut input = app.make_prediction_input();

    // Number of retries if the generated word is already in the learning database
    input.nb_try = 100;

    // Maximum n-gram size to consider during generation
    input.max_n = 0;

    // Start seed can be set to
    // 'False' to disable the seeding mechanism
    // 'Random' uses a random value of size n from the model
    // 'Custom' uses a custom string as the seed
    input.start_seed = StartSeed::False;

    // If the current internal key is not found in the database,
    // it will be reduced by 1. If 'reduce_random' is true,
    // the reduction can be random (results will be very random)
    input.reduce_random = false;

    // Set the intensity for each model (not normalized; will be computed in 'models_probability')
    input.set_model_intensity("french", 100.0)?;
    input.set_model_intensity("fromage", 50.0)?;
    input.set_model_intensity("ville", 25.0)?;
    input.set_model_intensity("pokemon", 0.0)?;

    // Attempting to set intensity for a non-existent model
    match input.set_model_intensity("unknown", 0.0) {
        Ok(_) => println!("Should not happen"),
        Err(_) => println!("This model ('unknown') does not exist"),
    }

    // Set the randomness factor for generation (must be between 0.0 and 1.0)
    input.set_randomness(0.1)?;

    // Test invalid randomness values
    match input.set_randomness(2.0) {
        Ok(_) => println!("Should not happen"),
        Err(_) => println!("Randomness 2.0 is invalid, must be between 0.0 and 1.0"),
    }
    match input.set_randomness(-1.0) {
        Ok(_) => println!("Should not happen"),
        Err(_) => println!("Randomness -1.0 is invalid, must be between 0.0 and 1.0"),
    }

    // The probability of each model is computed based on its intensity
    // The sum of all probabilities should be 1.0
    for (name, prob) in input.models_probability() {
        println!("{}: {}", name, prob);
    }

    // Generate 10 words using the input settings
    for i in 0..10 {
        println!("Generated word {}: {}", i + 1, app.predict(&input)?);
    }

    Ok(())
}
