const BASE_URL = "http://127.0.0.1:5000";
//const BASE_URL = "https://zetasoft.fr/generator/api";
const SETTINGS_KEY = "markov_generator_v2.0";

let isInitializing = true;
let lastGeneratedUrl = null;

/* =============================
   DOM References
============================= */
const terminal = document.getElementById("terminal");
const generateBtn = document.getElementById("generate-btn");
const copyBtn = document.getElementById("copy-command");
const openSettingsBtn = document.getElementById("open-settings");
const closeSettingsBtn = document.getElementById("close-settings");
const settingsModal = document.getElementById("settings-modal");

const randomness = document.getElementById("randomness");
const max_n = document.getElementById("max_n");
const nb_try = document.getElementById("nb_try");
const reduceRandom = document.getElementById("reduce_random");

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
    const seedValue = document.querySelector('input[name="seed"]:checked')?.value || "none";
    let seedQuery = {
        "custom": `custom:${encodeURIComponent(customSeed.value)}`,
        "random": `random:${parseInt(randomSeed.value)}`,
        "full-random": "random:0"
    }[seedValue] || "none";

    const intensityQuery = Array.from(modelContainer.querySelectorAll("input[type=range]"))
        .map(slider => `${encodeURIComponent(slider.dataset.model)}:${slider.value}`)
        .join(",");

    return `${BASE_URL}/v1/generate?max_n=${max_n.value}&nb_try=${nb_try.value}&randomness=${randomness.value}&reduce_random=${reduceRandom.checked}&seed=${seedQuery}&intensity=${intensityQuery}`;
}

function updateUrl() {
    lastGeneratedUrl = buildUrl();
    copyBtn.disabled = !lastGeneratedUrl;
}

/* =============================
   Settings Load/Save
============================= */
function loadSettings() {
    const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY)) || defaultSettings;

    Object.assign(randomness, { value: saved.randomness });
    Object.assign(max_n, { value: saved.max_n });
    Object.assign(nb_try, { value: saved.nb_try });
    reduceRandom.checked = saved.reduce_random;

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
        max_n: max_n.value,
        nb_try: nb_try.value,
        reduce_random: reduceRandom.checked,
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
    bindSlider(randomness, document.getElementById("randomness-val"), v => `${parseInt(v * 100)}%`);
    bindSlider(nb_try, document.getElementById("nb_try-val"));
    bindSlider(max_n, document.getElementById("max_n-val"), v => v === 0 ? "no limit" : v === 1 ? 2 : v);
}

/* =============================
   Models Loader
============================= */
async function loadModels() {
    try {
        const response = await fetch(`${BASE_URL}/v1/models`);
        const models = (await response.text()).trim().split("\n").filter(Boolean);
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
            slider.min = 0; slider.max = 100; slider.step = 0.1;
            slider.dataset.model = model;
            slider.value = saved.models?.[model] ?? 100;

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
        modelContainer.textContent = "Failed to load models";
    }
}

/* =============================
   Terminal Typewriter
============================= */
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
    generateBtn.disabled = true;
    try {
        const response = await fetch(lastGeneratedUrl);
        typeWriter(await response.text());
    } catch (err) {
        typeWriter("Error: " + err.message);
    }
    generateBtn.disabled = false;
}

/* =============================
   Terminal Init
============================= */
function initTerminal() {
    terminal.innerHTML = "";

    const loginLine = document.createElement("div");
    loginLine.className = "terminal-line terminal-muted";
    loginLine.textContent = (() => {
        const d = new Date();
        const days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        return `Last login: ${days[d.getDay()]} ${months[d.getMonth()]} ${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
    })();

    const promptLine = document.createElement("div");
    promptLine.className = "terminal-line";

    const user = document.createElement("span");
    user.className = "terminal-user";
    user.textContent = "rs-gen@zetasoft";

    const path = document.createElement("span");
    path.className = "terminal-path";
    path.textContent = " ~ $ ";

    const command = document.createElement("span");
    command.className = "terminal-line";
    command.textContent = ".\\rs-gen";

    promptLine.append(user, path, command);
    terminal.append(loginLine, promptLine);
}

/* =============================
   Event Listeners
============================= */
generateBtn.addEventListener("click", generate);
[randomness, max_n, nb_try, reduceRandom, customSeed, randomSeed].forEach(el => el.addEventListener("input", saveSettings));

seedRadios.forEach(r => r.addEventListener("change", () => {
    customSeed.disabled = r.value !== "custom";
    randomSeed.disabled = r.value !== "random";
    saveSettings();
}));

openSettingsBtn.addEventListener("click", () => settingsModal.classList.remove("hidden"));
closeSettingsBtn.addEventListener("click", () => settingsModal.classList.add("hidden"));
settingsModal.addEventListener("mousedown", (e) => {
    if (!document.querySelector(".modal-content").contains(e.target)) {
        settingsModal.classList.add("hidden");
    }
});
copyBtn.addEventListener("click", async () => {
    if (!lastGeneratedUrl) return;
    try {
        await navigator.clipboard.writeText(lastGeneratedUrl);
        copyBtn.textContent = "Copied!"; copyBtn.disabled = true;
        setTimeout(() => { copyBtn.textContent = "Copy URL"; copyBtn.disabled = false; }, 1000);
    } catch {
        copyBtn.textContent = "Error"; setTimeout(() => copyBtn.textContent = "Copy URL", 1000);
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
