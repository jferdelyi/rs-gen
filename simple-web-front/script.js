const BASE_URL = "https://zetasoft.fr/generator/api";
// const BASE_URL = "http://127.0.0.1:5000";

const SETTINGS_KEY = "markov_word_generator_v2.0";

let isInitializing = true;
let currentUrl = null;
// Display names of the models, by name, from GET /v1/labels
let modelLabels = {};

/* =============================
   DOM References
============================= */
const terminal = document.getElementById("terminal");
const announcer = document.getElementById("announcer");
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
const weights = document.getElementById("weights");
const weightsDialogTitle = document.getElementById("weights-dialog-title");
const weightsPanel = document.getElementById("weights-panel");
const equalWeightsBtn = document.getElementById("equal-weights");
const wordPanelBody = document.getElementById("word-panel-body");
const wordBubble = document.getElementById("word-bubble");

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
// Display names of the models, by name: none if the server does not give them
async function loadLabels() {
    try {
        const response = await fetch(`${BASE_URL}/v1/labels`);
        return response.ok ? await response.json() : {};
    } catch {
        return {};
    }
}

async function loadModels() {
    try {
        const response = await fetch(`${BASE_URL}/v1/models`);
        const models = (await response.text())
            .trim()
            .split("\n")
            .filter(Boolean);

        const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY)) || defaultSettings;
        const labels = await loadLabels();
        modelLabels = labels;

        modelContainer.innerHTML = "";

        models.forEach(model => {
            const wrapper = document.createElement("div");
            wrapper.className = "input-group";

            const slider = document.createElement("input");
            // Model names are letters, digits, '_' and '-': a valid id as they are
            slider.id = `weight-${model}`;

            const label = document.createElement("label");
            label.className = "label-title";
            label.htmlFor = slider.id;
            // The display name for the visitor; the API keeps the technical name
            label.textContent = labels[model] || model;

            const row = document.createElement("div");
            row.className = "slider-inline";

            slider.type = "range";
            slider.min = 0;
            slider.max = 100;
            slider.step = 0.1;
            slider.dataset.model = model;
            slider.value = saved.models?.[model] ?? 100;
            slider.title = `Adjust the weight of model "${labels[model] || model}"`;

            const valueSpan = document.createElement("span");
            valueSpan.className = "slider-value";
            valueSpan.textContent = `${slider.value}%`;

            slider.addEventListener("input", () => {
                valueSpan.textContent = `${slider.value}%`;
                saveSettings();
                // A weight moved by hand: the weights from before Equal weights are forgotten
                if (!settingWeights) {
                    forgetWeightsBeforeEqual();
                }
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
   Weights: in the settings dialog, or in their panel on a wide screen
============================= */
// One set of sliders, moved from one place to the other: nothing to keep in step.
// Same width as the wide screen block of style.css
const wideScreen = window.matchMedia("(min-width: 1000px)");

function placeWeights() {
    if (wideScreen.matches) {
        weightsPanel.appendChild(weights);
    } else {
        weightsDialogTitle.after(weights);
    }
    weightsDialogTitle.classList.toggle("hidden", wideScreen.matches);
}

// The weights from before Equal weights, by model, to put them back; none once a weight is
// moved by hand
let weightsBeforeEqual = null;
// True while the page sets the weights itself, not the visitor
let settingWeights = false;

const EQUAL_LABEL = "Equal weights";
const EQUAL_TIP = "Give every model the same weight, 100%";
const UNDO_LABEL = "Undo equal weights";
const UNDO_TIP = "Put back the weights from before Equal weights";

// Sets the weight of each slider, then saves the settings once per slider
function setWeights(weightOf) {
    settingWeights = true;
    modelContainer.querySelectorAll("input[type=range]").forEach(slider => {
        slider.value = weightOf(slider.dataset.model, slider.value);
        slider.dispatchEvent(new Event("input"));
    });
    settingWeights = false;
}

function forgetWeightsBeforeEqual() {
    weightsBeforeEqual = null;
    equalWeightsBtn.textContent = EQUAL_LABEL;
    equalWeightsBtn.title = EQUAL_TIP;
}

// Every model at the same weight, 100%, as on a first visit; a second press puts the weights
// from before back
function equalWeights() {
    if (weightsBeforeEqual) {
        const before = weightsBeforeEqual;
        setWeights((model, value) => before[model] ?? value);
        forgetWeightsBeforeEqual();
        return;
    }
    const before = Object.fromEntries(
        Array.from(modelContainer.querySelectorAll("input[type=range]")).map(slider => [slider.dataset.model, slider.value])
    );
    setWeights(() => 100);
    weightsBeforeEqual = before;
    equalWeightsBtn.textContent = UNDO_LABEL;
    equalWeightsBtn.title = UNDO_TIP;
}

/* =============================
   Terminal
============================= */
function initTerminal() {
    // The bubble may sit in a line of the terminal: it is kept
    document.body.appendChild(wordBubble);
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
    forgetWords();
}

// Announces a whole text to screen readers; emptied first, so that the same word twice is
// announced twice
function announce(text) {
    announcer.textContent = "";
    setTimeout(() => {
        announcer.textContent = text;
    }, 50);
}

function typeWriter(text) {
    announce(text);
    const line = document.createElement("div");
    terminal.appendChild(line);

    let i = 0;
    const interval = setInterval(() => {
        line.textContent += text[i++];
        terminal.scrollTop = terminal.scrollHeight;
        if (i >= text.length) clearInterval(interval);
    }, 15);
}

// Writes a word letter by letter, as typeWriter, in an element that keeps its trace
function typeWord(trace) {
    announce(trace.word);
    const line = document.createElement("div");
    const word = element("span", "word");
    line.appendChild(word);
    terminal.appendChild(line);

    const letters = [...trace.word];
    const typed = () => {
        watchWord(word, trace);
        // On a very wide screen, the panel shows the last word
        if (veryWideScreen.matches) {
            showWord(word);
        }
    };
    if (letters.length === 0) {
        typed();
        return;
    }
    let i = 0;
    const interval = setInterval(() => {
        word.textContent += letters[i++];
        terminal.scrollTop = terminal.scrollHeight;
        if (i >= letters.length) {
            clearInterval(interval);
            typed();
        }
    }, 15);
}

/* =============================
   How a word was made: the trace of GET /v1/generate?trace=true, drawn in the Last Word
   panel on a very wide screen, in a bubble under the word otherwise
============================= */
// Same width as the very wide screen block of style.css
const veryWideScreen = window.matchMedia("(min-width: 1360px)");

// The model with the largest share of a word takes the first color, up to the eighth
const MODEL_COLORS = 8;
// Circumference of the circles of the pie, of radius 8
const PIE = 2 * Math.PI * 8;
const SVG = "http://www.w3.org/2000/svg";
// A word hovered with the mouse shows its bubble after this delay, as the tooltips; leaving
// it hides the bubble after a grace delay, the time to reach the bubble
const BUBBLE_SHOW_MS = 400;
const BUBBLE_HIDE_MS = 300;

// The trace of each word of the terminal
const wordTraces = new WeakMap();
// The word drawn in the panel or in the bubble
let shownWord = null;
let bubbleTimer = null;
// A focus given back to a word by Escape does not open its bubble again
let quietFocus = false;

function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
}

function labelOf(model) {
    return modelLabels[model] || model;
}

function percent(part, whole) {
    return `${Math.round(part / whole * 100)}%`;
}

// The models of a word with their number of letters, the largest share first, and their color
function modelShares(trace) {
    const counts = new Map();
    for (const letter of trace.letters) {
        counts.set(letter.model, (counts.get(letter.model) || 0) + 1);
    }
    return [...counts]
        .sort((a, b) => b[1] - a[1] || labelOf(a[0]).localeCompare(labelOf(b[0])))
        .map(([model, count], i) => ({ model, count, color: Math.min(i + 1, MODEL_COLORS) }));
}

// The settings a word was generated with, told under its trace
function settingsOf() {
    const seed = {
        custom: `"${customSeed.value}"`,
        random: `random, order ${randomSeed.value}`,
        "full-random": "random",
    }[document.querySelector('input[name="seed"]:checked')?.value] || "none";
    return {
        randomness: `${Math.round(Number(randomness.value) * 100)}%`,
        maxN: Number(maxN.value) === 0 ? "none" : maxN.value,
        seed,
        reduceRandom: progressiveRandomness.checked,
    };
}

// A cell of the trace, with its classes and the tooltip that explains it, if any
function cell(row, text, className, tip) {
    const td = row.insertCell();
    if (className) td.className = className;
    if (tip) td.title = tip;
    td.textContent = text;
    return td;
}

// The letters looked at, in words: "the start of the word", "1 letter", "4 letters"
function lettersLooked(step) {
    const count = [...step.context].length;
    if (count === 0) return "the start of the word";
    return `${count} ${count === 1 ? "letter" : "letters"}`;
}

// What happened while a letter, or the end, was chosen; `before` is the number of letters
// before it in the word
function events(step, isEnd, before) {
    const found = [];
    const fallbacks = step.fallbacks.map(labelOf);
    if (fallbacks.length === 1) {
        found.push([`${fallbacks[0]} had never seen these letters`, "trace-warning", "This model was drawn first: the next one was tried"]);
    } else if (fallbacks.length > 1) {
        found.push([`${fallbacks.length} models had never seen these letters`, "trace-warning", `Drawn first, one after the other: ${fallbacks.join(", ")}`]);
    }
    // The letters first looked at, the start of the word not counted
    const first = step.reductions.length > 0 ? Math.min(step.reductions[0] - 1, before) : 0;
    if (step.reductions.length > 0 && step.random_n) {
        found.push([`Randomness drew ${first} letters, no model knew them all: looked at ${lettersLooked(step)}`, "trace-danger", "Fewer letters were looked at, until a model knew them"]);
    } else if (step.reductions.length > 0) {
        found.push([`no model knew the last ${first} letters: looked at ${lettersLooked(step)}`, "trace-danger", "No model had seen them all: fewer letters were looked at"]);
    } else if (step.random_n) {
        found.push([`looked at ${lettersLooked(step)} only (Randomness)`, "trace-warning", "The Randomness setting drew how many letters to look at"]);
    }
    if (isEnd) {
        found.push(["end of the word", "trace-muted", ""]);
    } else if (step.choices === 1) {
        found.push(["the only letter ever seen here", "trace-muted", "Nothing was drawn"]);
    } else if (found.length === 0) {
        found.push([`1 of ${step.choices} possible letters`, "trace-muted", ""]);
    }
    return found;
}

// The pie of the shares of the models; the legend says the same in words
function pieChart(shares, letters) {
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("class", "trace-pie");
    svg.setAttribute("viewBox", "0 0 32 32");
    svg.setAttribute("aria-hidden", "true");
    const group = document.createElementNS(SVG, "g");
    group.setAttribute("transform", "rotate(-90 16 16)");
    let offset = 0;
    for (const share of shares) {
        const length = share.count / letters * PIE;
        const slice = document.createElementNS(SVG, "circle");
        slice.setAttribute("class", `slice model-${share.color}`);
        slice.setAttribute("cx", "16");
        slice.setAttribute("cy", "16");
        slice.setAttribute("r", "8");
        slice.setAttribute("stroke-dasharray", `${length} ${PIE}`);
        slice.setAttribute("stroke-dashoffset", `${-offset}`);
        group.appendChild(slice);
        offset += length;
    }
    svg.appendChild(group);
    return svg;
}

// The columns of the trace, with what each one tells
const TRACE_COLUMNS = [
    ["#", "col-index", "Position of the letter in the word"],
    ["letter", "", "The letter, in the color of the model that gave it"],
    ["model", "", "The model that gave the letter"],
    ["after", "", "The letters looked at to choose this one; ^ is the start of the word"],
    ["chance", "", "How often this letter came after them in the training words of the model"],
    ["event", "col-event", "What happened while this letter was chosen"],
];

// How to read a trace: shown on demand, read on a phone as well, where there is no tooltip
const TRACE_HELP = [
    "Each letter comes from one model, drawn at random according to the weights.",
    "after: the letters the model looked at before choosing; ^ is the start of the word.",
    "chance: how often this letter came after them in the training words of the model.",
    "had never seen these letters: the model drawn first knew nothing after them, so the next one was tried.",
    "no model knew the last N letters: none had seen them all, so fewer letters were looked at.",
    "looked at N letters only (Randomness): the Randomness setting drew how many letters to look at.",
    "the only letter ever seen here: there was no choice to draw.",
    "Refused first: words generated before this one, already in the training files (Retry Attempts).",
];

// Draws how a word was made in `container`, as a terminal would: the word in the colors of its
// models, their shares, each letter, the settings, the buttons
function renderTrace(trace, container) {
    const shares = modelShares(trace);
    const colorOf = model => shares.find(share => share.model === model)?.color;

    const word = element("div", "trace-word");
    const seed = trace.seed ? [...trace.seed.text] : [];
    for (const letter of seed) {
        const node = element("span", "trace-seed", letter);
        node.title = "Given as the seed";
        word.appendChild(node);
    }
    for (const letter of trace.letters) {
        word.appendChild(element("span", `model-${colorOf(letter.model)}`, letter.letter));
    }
    const head = element("div", "trace-head");
    head.appendChild(word);
    if (shares.length > 0) {
        const legend = element("ul", "trace-legend");
        for (const share of shares) {
            const item = element("li");
            item.title = `${share.count} of the ${trace.letters.length} letters`;
            const swatch = element("span", `swatch model-${share.color}`);
            swatch.setAttribute("aria-hidden", "true");
            item.append(swatch, `${labelOf(share.model)} ${percent(share.count, trace.letters.length)}`);
            legend.appendChild(item);
        }
        const key = element("div", "trace-key");
        key.append(legend, pieChart(shares, trace.letters.length));
        head.appendChild(key);
    }

    const table = element("table", "trace-table");
    table.setAttribute("aria-label", "How each letter was chosen");
    const header = table.createTHead().insertRow();
    for (const [title, className, tip] of TRACE_COLUMNS) {
        const th = element("th", className, title);
        th.scope = "col";
        th.title = tip;
        header.appendChild(th);
    }
    const body = table.createTBody();
    const rows = trace.letters.map((letter, i) => [String(seed.length + i + 1), letter.letter, `model-${colorOf(letter.model)}`, letter, false, seed.length + i]);
    rows.push(["", "⏎", "trace-muted", trace.end, true, seed.length + trace.letters.length]);
    for (const [index, letter, letterClass, step, isEnd, before] of rows) {
        const row = body.insertRow();
        cell(row, index, "col-index trace-muted");
        cell(row, letter, letterClass, isEnd ? "End of the word" : "");
        cell(row, labelOf(step.model));
        // The letters looked at: the last 6 only when they are many, all of them in the tooltip
        const context = [...step.context];
        const shown = context.length > 7 ? `…${context.slice(-6).join("")}` : `${step.at_start ? "^" : ""}${step.context}`;
        cell(row, shown, "", `Looked at ${step.at_start ? "the start of the word" : ""}${step.at_start && context.length > 0 ? " and " : ""}${context.length > 0 ? `"${step.context}"` : ""}`);
        cell(row, percent(step.count, step.total), "",
            `${step.count} of ${step.total} times, among ${step.choices} possible ${step.choices === 1 ? "letter" : "letters"}`);
        // The event, in its column; on a narrow trace, on its own line under the letter
        const event = cell(row, "", "col-event");
        const eventLine = body.insertRow();
        eventLine.className = "event-row";
        const eventBelow = cell(eventLine, "");
        eventBelow.colSpan = TRACE_COLUMNS.length;
        for (const target of [event, eventBelow]) {
            events(step, isEnd, before).forEach(([text, className, tip], i) => {
                if (i > 0) target.append(" · ");
                const span = element("span", className, text);
                if (tip) span.title = tip;
                target.appendChild(span);
            });
        }
    }

    const parts = [head, table];
    if (trace.rejected.length > 0) {
        parts.push(element("p", "trace-attempts", `Refused first, already in the training files: ${trace.rejected.join(", ")}`));
    }
    if (trace.settings) {
        const s = trace.settings;
        const seedFrom = trace.seed?.model ? `, drawn from ${labelOf(trace.seed.model)}` : "";
        const settings = [`randomness ${s.randomness}`, `max order ${s.maxN}`, `seed ${s.seed}${seedFrom}`];
        if (s.reduceRandom) settings.push("advanced randomness");
        parts.push(element("p", "trace-attempts", `Settings: ${settings.join(" · ")}`));
    }

    const help = element("details", "trace-help");
    const list = element("ul");
    for (const line of TRACE_HELP) {
        list.appendChild(element("li", "", line));
    }
    help.append(element("summary", "", "How to read this"), list);
    parts.push(help);

    const buttons = element("div", "trace-buttons");
    const copy = element("button", "term-btn", "[ copy ]");
    copy.title = "Copy the word";
    copy.addEventListener("click", async () => {
        try {
            await navigator.clipboard.writeText(trace.word);
            copy.textContent = "[ copied ]";
        } catch {
            copy.textContent = "[ copy failed ]";
        }
        setTimeout(() => {
            copy.textContent = "[ copy ]";
        }, 1000);
    });
    // The definition of the word by an AI, to come: off, but reachable for its tooltip
    const man = element("button", "term-btn", "[ man ]");
    man.title = "AI definition, coming soon";
    man.setAttribute("aria-disabled", "true");
    buttons.append(copy, man);
    parts.push(buttons);

    // The bubble can fill a phone: its close button stays at the top while it scrolls
    if (container === wordBubble) {
        const bar = element("div", "trace-bar");
        const close = element("button", "term-btn", "[ close ]");
        close.title = "Close (Escape)";
        close.addEventListener("click", () => {
            hideBubble();
            if (shownWord) {
                quietFocus = true;
                shownWord.focus();
            }
        });
        bar.appendChild(close);
        parts.unshift(bar);
    }

    container.replaceChildren(...parts);
}

// Draws a word in the panel on a very wide screen, in the bubble under it otherwise
function showWord(word) {
    clearTimeout(bubbleTimer);
    const trace = wordTraces.get(word);
    if (!trace) return;
    shownWord = word;
    if (veryWideScreen.matches) {
        renderTrace(trace, wordPanelBody);
        return;
    }
    renderTrace(trace, wordBubble);
    // Right after its word in the page: Tab goes from the word to the buttons of its bubble
    word.after(wordBubble);
    wordBubble.classList.remove("hidden");
    placeBubble(word);
}

function hideBubble() {
    clearTimeout(bubbleTimer);
    wordBubble.classList.add("hidden");
}

// Under the word, or above it when the screen has no room below; always inside the screen
function placeBubble(word) {
    const margin = 8;
    const box = word.getBoundingClientRect();
    const width = wordBubble.offsetWidth;
    const height = wordBubble.offsetHeight;
    const below = box.bottom + 6;
    const top = below + height + margin <= window.innerHeight ? below : Math.max(margin, box.top - 6 - height);
    const left = Math.max(margin, Math.min(box.left, window.innerWidth - width - margin));
    wordBubble.style.left = `${left}px`;
    wordBubble.style.top = `${top}px`;
}

// A word of the terminal shows how it was made: after a delay on a mouse hover, at once on a
// keyboard focus or a tap
function watchWord(word, trace) {
    wordTraces.set(word, trace);
    word.tabIndex = 0;
    word.addEventListener("pointerenter", event => {
        if (event.pointerType !== "mouse") return;
        clearTimeout(bubbleTimer);
        if (veryWideScreen.matches) {
            showWord(word);
        } else {
            bubbleTimer = setTimeout(() => showWord(word), BUBBLE_SHOW_MS);
        }
    });
    word.addEventListener("pointerleave", event => {
        if (event.pointerType !== "mouse" || veryWideScreen.matches) return;
        clearTimeout(bubbleTimer);
        bubbleTimer = setTimeout(hideBubble, BUBBLE_HIDE_MS);
    });
    word.addEventListener("focusin", () => {
        if (quietFocus) {
            quietFocus = false;
            return;
        }
        showWord(word);
    });
    word.addEventListener("click", () => showWord(word));
}

// The terminal is cleared: no word to show
function forgetWords() {
    hideBubble();
    shownWord = null;
    wordPanelBody.replaceChildren(element("p", "trace-empty", "Generate a word to see how it was made."));
}

/* =============================
   Generate
============================= */
// Answer of the server when the request names a model it does not serve
const UNKNOWN_MODEL = /^Model .+ not found$/;

// Reads the list of models again: the admin may have hidden, shown or added one
async function reloadModels() {
    await loadModels();
    updateUrl();
}

// Asks a word with its trace; an error comes back as text
async function requestWord() {
    const response = await fetch(`${currentUrl}&trace=true`);
    if (!response.ok) {
        return { status: response.status, error: await response.text() };
    }
    return { trace: await response.json() };
}

async function generate() {
    if (!currentUrl) return;

    generateBtn.disabled = true;

    try {
        let result = await requestWord();
        // A model was hidden or deleted since the page loaded: retry once with the new list
        if (result.status === 400 && UNKNOWN_MODEL.test(result.error.trim())) {
            await reloadModels();
            result = await requestWord();
        }
        if (result.trace) {
            result.trace.settings = settingsOf();
            typeWord(result.trace);
        } else {
            typeWriter(result.error);
        }
    } catch (err) {
        typeWriter(`Error: ${err.message}`);
    }

    generateBtn.disabled = false;
}

/* =============================
   Event Listeners
============================= */
generateBtn.addEventListener("click", generate);

// Coming back to the page shows the models the server serves now
document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible" && !isInitializing) {
        reloadModels();
    }
});

[randomness, maxN, retryCount, progressiveRandomness, customSeed, randomSeed]
    .forEach(el => el.addEventListener("input", saveSettings));

seedRadios.forEach(radio => {
    radio.addEventListener("change", () => {
        customSeed.disabled = radio.value !== "custom";
        randomSeed.disabled = radio.value !== "random";
        saveSettings();
    });
});

// A modal dialog: the page behind stops answering and Escape closes it
openSettingsBtn.addEventListener("click",
    () => settingsModal.showModal()
);

closeSettingsBtn.addEventListener("click",
    () => settingsModal.close()
);

resetBtn.addEventListener("click", initTerminal);

equalWeightsBtn.addEventListener("click", equalWeights);

wideScreen.addEventListener("change", placeWeights);

// Leaving the very wide screen moves the word from the panel to a bubble, and back
veryWideScreen.addEventListener("change", () => {
    hideBubble();
    if (veryWideScreen.matches && shownWord) {
        showWord(shownWord);
    }
});

// The bubble stays while the pointer is on it
wordBubble.addEventListener("pointerenter", () => clearTimeout(bubbleTimer));
wordBubble.addEventListener("pointerleave", event => {
    if (event.pointerType !== "mouse") return;
    bubbleTimer = setTimeout(hideBubble, BUBBLE_HIDE_MS);
});

// Escape closes the bubble; a focus inside it goes back to its word
document.addEventListener("keydown", event => {
    if (event.key !== "Escape" || wordBubble.classList.contains("hidden")) return;
    const inside = wordBubble.contains(document.activeElement);
    hideBubble();
    if (inside && shownWord) {
        quietFocus = true;
        shownWord.focus();
    }
});

// A focus or a press away from the word and its bubble closes it
document.addEventListener("focusin", event => {
    if (wordBubble.classList.contains("hidden")) return;
    if (event.target !== shownWord && !wordBubble.contains(event.target)) {
        hideBubble();
    }
});
document.addEventListener("pointerdown", event => {
    if (wordBubble.classList.contains("hidden")) return;
    if (!wordBubble.contains(event.target) && !event.target.closest?.(".word")) {
        hideBubble();
    }
});

// The bubble is placed for the word where it was: a scroll or a resize closes it
terminal.addEventListener("scroll", hideBubble);
window.addEventListener("resize", hideBubble);

// The dialog covers the screen around its card: a press on the dialog itself is outside the card
settingsModal.addEventListener("mousedown", e => {
    if (e.target === settingsModal) {
        settingsModal.close();
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
    placeWeights();
    await loadModels();
    loadSettings();
    initTerminal();
    isInitializing = false;
}

init();
