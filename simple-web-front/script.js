const BASE_URL = "https://zetasoft.fr/generator/api";
// const BASE_URL = "http://127.0.0.1:5000";

const SETTINGS_KEY = "markov_word_generator_v2.0";

let isInitializing = true;
let currentUrl = null;

/* =============================
   DOM References
============================= */
const terminal = document.getElementById("terminal");
const generateBtn = document.getElementById("generate-btn");
const copyBtn = document.getElementById("copy-command");
const openSettingsBtn = document.getElementById("open-settings");
const resetBtn = document.getElementById("reset-btn");
const closeSettingsBtn = document.getElementById("close-settings");
const settingsModal = document.getElementById("settings-modal");

const randomness = document.getElementById("randomness");
const maxN = document.getElementById("max_n");
const retryCount = document.getElementById("nb_try");
const progressiveRandomness = document.getElementById("reduce_random");

const seedRadios = document.querySelectorAll('input[name="seed"]');
const customSeed = document.getElementById("custom-seed");
const randomSeed = document.getElementById("random-seed");

const modelContainer = document.getElementById("model-intensities");

/* =============================
   Default Settings
============================= */
const defaultSettings = {
    randomness: 0.1,
    max_n: 0,
    nb_try: 100,
    reduce_random: false,
    seed: "none",
    custom_seed: "",
    random_seed: 2,
    models: {}
};

/* =============================
   URL Builder
============================= */
function buildUrl() {
    const selectedSeed = document.querySelector('input[name="seed"]:checked')?.value || "none";

    const seedQuery = {
        custom: `custom:${encodeURIComponent(customSeed.value)}`,
        random: `random:${parseInt(randomSeed.value)}`,
        "full-random": "random:0"
    }[selectedSeed] || "none";

    const intensityQuery = Array.from(
        modelContainer.querySelectorAll("input[type=range]")
    )
        .map(slider => `${encodeURIComponent(slider.dataset.model)}:${slider.value}`)
        .join(",");

    return `${BASE_URL}/v1/generate?max_n=${maxN.value}&nb_try=${retryCount.value}&randomness=${randomness.value}&reduce_random=${progressiveRandomness.checked}&seed=${seedQuery}&intensity=${intensityQuery}`;
}

function updateUrl() {
    currentUrl = buildUrl();
    copyBtn.disabled = !currentUrl;
}

/* =============================
   Settings Load / Save
============================= */
function loadSettings() {
    const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY)) || defaultSettings;

    randomness.value = saved.randomness;
    maxN.value = saved.max_n;
    retryCount.value = saved.nb_try;
    progressiveRandomness.checked = saved.reduce_random;

    seedRadios.forEach(r => r.checked = r.value === saved.seed);

    customSeed.value = saved.custom_seed;
    randomSeed.value = saved.random_seed;

    updateUrl();
    bindSliders();
}

function saveSettings() {
    if (isInitializing) return;

    const models = Object.fromEntries(
        Array.from(modelContainer.querySelectorAll("input[type=range]"))
            .map(slider => [slider.dataset.model, slider.value])
    );

    const selectedSeed = document.querySelector('input[name="seed"]:checked')?.value;

    const settings = {
        randomness: randomness.value,
        max_n: maxN.value,
        nb_try: retryCount.value,
        reduce_random: progressiveRandomness.checked,
        seed: selectedSeed,
        custom_seed: customSeed.value,
        random_seed: randomSeed.value,
        models
    };

    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    updateUrl();
}

/* =============================
   Slider Binding
============================= */
function bindSlider(slider, output, formatter = v => v) {
    const update = () => {
        output.textContent = formatter(Number(slider.value));
        saveSettings();
    };
    slider.addEventListener("input", update);
    update();
}

function bindSliders() {
    bindSlider(randomness,
        document.getElementById("randomness-val"),
        v => `${Math.round(v * 100)}%`
    );

    bindSlider(retryCount,
        document.getElementById("nb_try-val")
    );

    bindSlider(maxN,
        document.getElementById("max_n-val"),
        v => v === 0 ? "No limit" : v
    );
}

/* =============================
   Models Loader
============================= */
async function loadModels() {
    try {
        const response = await fetch(`${BASE_URL}/v1/models`);
        const models = (await response.text())
            .trim()
            .split("\n")
            .filter(Boolean);

        const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY)) || defaultSettings;

        modelContainer.innerHTML = "";

        models.forEach(model => {
            const wrapper = document.createElement("div");
            wrapper.className = "input-group";

            const label = document.createElement("label");
            label.className = "label-title";
            label.textContent = model;

            const row = document.createElement("div");
            row.className = "slider-inline";

            const slider = document.createElement("input");
            slider.type = "range";
            slider.min = 0;
            slider.max = 100;
            slider.step = 0.1;
            slider.dataset.model = model;
            slider.value = saved.models?.[model] ?? 100;
            slider.title = `Adjust the weight of model "${model}"`;

            const valueSpan = document.createElement("span");
            valueSpan.className = "slider-value";
            valueSpan.textContent = `${slider.value}%`;

            slider.addEventListener("input", () => {
                valueSpan.textContent = `${slider.value}%`;
                saveSettings();
            });

            row.append(slider, valueSpan);
            wrapper.append(label, row);
            modelContainer.appendChild(wrapper);
        });

    } catch {
        modelContainer.textContent = "Unable to load models.";
    }
}

/* =============================
   Terminal
============================= */
function initTerminal() {
    terminal.innerHTML = "";

    const loginLine = document.createElement("div");
    loginLine.className = "terminal-line terminal-muted";

    const d = new Date();
    loginLine.textContent =
        `Last login: ${d.toLocaleString("en-US", {
            weekday: "short",
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
            second: "2-digit",
            hour12: false
        })}`;

    const promptLine = document.createElement("div");
    promptLine.className = "terminal-line";

    const user = document.createElement("span");
    user.className = "terminal-user";
    user.textContent = "rs-gen@zetasoft";

    const path = document.createElement("span");
    path.className = "terminal-path";
    path.textContent = " ~ $ ";

    const command = document.createElement("span");
    command.textContent = "./rs-gen";

    promptLine.append(user, path, command);
    terminal.append(loginLine, promptLine);
}

function typeWriter(text) {
    const line = document.createElement("div");
    terminal.appendChild(line);

    let i = 0;
    const interval = setInterval(() => {
        line.textContent += text[i++];
        terminal.scrollTop = terminal.scrollHeight;
        if (i >= text.length) clearInterval(interval);
    }, 15);
}

/* =============================
   Generate
============================= */
async function generate() {
    if (!currentUrl) return;

    generateBtn.disabled = true;

    try {
        const response = await fetch(currentUrl);
        const result = await response.text();
        typeWriter(result);
    } catch (err) {
        typeWriter(`Error: ${err.message}`);
    }

    generateBtn.disabled = false;
}

/* =============================
   Event Listeners
============================= */
generateBtn.addEventListener("click", generate);

[randomness, maxN, retryCount, progressiveRandomness, customSeed, randomSeed]
    .forEach(el => el.addEventListener("input", saveSettings));

seedRadios.forEach(radio => {
    radio.addEventListener("change", () => {
        customSeed.disabled = radio.value !== "custom";
        randomSeed.disabled = radio.value !== "random";
        saveSettings();
    });
});

openSettingsBtn.addEventListener("click",
    () => settingsModal.classList.remove("hidden")
);

closeSettingsBtn.addEventListener("click",
    () => settingsModal.classList.add("hidden")
);

resetBtn.addEventListener("click", initTerminal);

settingsModal.addEventListener("mousedown", e => {
    if (!document.querySelector(".modal-content").contains(e.target)) {
        settingsModal.classList.add("hidden");
    }
});

copyBtn.addEventListener("click", async () => {
    if (!currentUrl) return;

    try {
        await navigator.clipboard.writeText(currentUrl);
        copyBtn.textContent = "Copied!";
        copyBtn.disabled = true;

        setTimeout(() => {
            copyBtn.textContent = "Copy API URL";
            copyBtn.disabled = false;
        }, 1000);

    } catch {
        copyBtn.textContent = "Copy failed";
        setTimeout(() => {
            copyBtn.textContent = "Copy API URL";
        }, 1000);
    }
});

/* =============================
   Init
============================= */
async function init() {
    await loadModels();
    loadSettings();
    initTerminal();
    isInitializing = false;
}

init();
