const BASE_URL = 'http://127.0.0.1:5000'; // To adapt to your deployment

const btn = document.getElementById('predict-btn');
const result = document.getElementById('result');
const modelsDiv = document.getElementById('models');
const commandDiv = document.getElementById('command');
const historyDiv = document.getElementById('history');

// Randomness
const randomnessSlider = document.getElementById('randomness-slider');
const randomnessVal = document.getElementById('randomness-val');
let randomness = parseFloat(randomnessSlider.value);
randomnessSlider.addEventListener('input', () => {
    randomness = parseFloat(randomnessSlider.value);
    randomnessVal.textContent = randomness.toFixed(2);
});

// Max n
const maxnSlider = document.getElementById('maxn-slider');
const maxnVal = document.getElementById('maxn-val');
let max_n = parseInt(maxnSlider.value);
maxnSlider.addEventListener('input', () => {
    max_n = parseInt(maxnSlider.value);
    maxnVal.textContent = max_n;
});

// nb_try
const nbtrySlider = document.getElementById('nbtry-slider');
const nbtryVal = document.getElementById('nbtry-val');
let nb_try = parseInt(nbtrySlider.value);
nbtrySlider.addEventListener('input', () => {
    nb_try = parseInt(nbtrySlider.value);
    nbtryVal.textContent = nb_try;
});

// reduce_random
const reduceRandomCheckbox = document.getElementById('reduce-random');

// start_seed
const seedRadios = document.querySelectorAll('input[name="seed"]');
const customSeedInput = document.getElementById('custom-seed');
const randomSeedInput = document.getElementById('random-seed');
let startSeed = 'none';
seedRadios.forEach(radio => {
    radio.addEventListener('change', () => {
        startSeed = document.querySelector('input[name="seed"]:checked').value;
        customSeedInput.disabled = startSeed !== 'custom';
        randomSeedInput.disabled = startSeed !== 'random';
    });
});

// model sliders
const modelSliders = {};
async function loadModels() {
    try {
        const response = await fetch(`${BASE_URL}/v1/models`);
        if (!response.ok) throw new Error(`API returned ${response.status}`);
        const text = await response.text();
        const models = text.trim().split('\n').filter(m => m.length > 0);

        if (models.length === 0) {
            modelsDiv.textContent = "No models loaded";
            return;
        }

        modelsDiv.innerHTML = '';
        models.forEach(model => {
            const container = document.createElement('div');
            container.className = 'slider-container';

            const label = document.createElement('div');
            label.className = 'slider-label';
            label.innerHTML = `<span>${model}</span><span id="val-${model}">100.0%</span>`;

            const slider = document.createElement('input');
            slider.type = 'range';
            slider.min = 0;
            slider.max = 100;
            slider.step = 0.1;
            slider.value = 100.0;
            slider.addEventListener('input', () => {
                document.getElementById(`val-${model}`).textContent = `${slider.value}%`;
                modelSliders[model] = slider.value;
            });

            container.appendChild(label);
            container.appendChild(slider);
            modelsDiv.appendChild(container);

            modelSliders[model] = 100.0;
        });

    } catch (err) {
        modelsDiv.textContent = `Error: ${err.message}`;
    }
}

loadModels();

btn.addEventListener('click', async () => {
    result.textContent = "Loading...";
    try {
        const intensityQuery = Object.entries(modelSliders)
            .map(([name, val]) => `${encodeURIComponent(name)}:${val}`)
            .join(',');

        let seedQuery = 'none';
        if (startSeed === 'custom') seedQuery = `custom:${encodeURIComponent(customSeedInput.value)}`;
        else if (startSeed === 'random') seedQuery = `random:${parseInt(randomSeedInput.value)}`;

        const url = `${BASE_URL}/v1/generate?max_n=${max_n}&nb_try=${nb_try}&randomness=${randomness}&reduce_random=${reduceRandomCheckbox.checked}&seed=${seedQuery}&intensity=${intensityQuery}`;

        commandDiv.textContent = url;

        const response = await fetch(url);
        if (!response.ok) throw new Error(`API returned ${response.status}`);
        const text = await response.text();

        result.textContent = `Prediction: ${text}`;
        historyDiv.textContent = text + "\n" + historyDiv.textContent;

    } catch (err) {
        result.textContent = `Error: ${err.message}`;
    }
});
