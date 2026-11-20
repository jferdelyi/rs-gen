//! Minimal example of the `rs-gen-core` API: it loads the models of `./data`, sets the generation
//! parameters and the weights of the models, shows how invalid values are refused, then prints
//! ten generated words. Run it from the root of the repository: `cargo run -p rs-gen-exemple`.

use rs_gen_core::model::generator::Generator;
use rs_gen_core::model::prediction_input::StartSeed;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Every model of `./data` (`.dat` files), read from its model file (`.bin`) when there is one
	let app: Generator = Generator::new("./data")?;

	// The generation parameters, with the default weight for every model
	let mut input = app.make_prediction_input();

	// Tries again, up to 100 times, when the generated word is already in a training file
	input.nb_try = 100;

	// Largest n-gram order used for the generation; 0 for no limit
	input.max_n = 0;

	// How a word starts:
	// - `StartSeed::None`: without a predefined prefix
	// - `StartSeed::Random(n)`: a random prefix of a model, bounded by the order n
	// - `StartSeed::Custom(text)`: the given prefix
	input.start_seed = StartSeed::None;

	// A prefix that no model knows is shortened by one character; with `reduce_random`, the
	// shortening is random and the words much more random
	input.reduce_random = false;

	// The weight of each model; they do not need to add up to anything (see `models_probability`)
	input.set_model_intensity("french", 100.0)?;
	input.set_model_intensity("fromage", 50.0)?;
	input.set_model_intensity("ville", 25.0)?;
	input.set_model_intensity("pokemon", 0.0)?;

	// A model that does not exist is refused
	match input.set_model_intensity("unknown", 0.0) {
		Ok(_) => println!("Should not happen"),
		Err(_) => println!("This model ('unknown') does not exist"),
	}

	// The randomness of the generation, between 0.0 and 1.0
	input.set_randomness(0.1)?;

	// Values out of range are refused
	match input.set_randomness(2.0) {
		Ok(_) => println!("Should not happen"),
		Err(_) => println!("Randomness 2.0 is invalid, must be between 0.0 and 1.0"),
	}
	match input.set_randomness(-1.0) {
		Ok(_) => println!("Should not happen"),
		Err(_) => println!("Randomness -1.0 is invalid, must be between 0.0 and 1.0"),
	}

	// The probability of each model, from its weight: they add up to 1.0
	for (name, prob) in input.models_probability() {
		println!("{name}: {prob}");
	}

	// Ten words generated with these parameters
	for i in 0..10 {
		println!("Generated word {}: {}", i + 1, app.predict(&input)?);
	}

	Ok(())
}
