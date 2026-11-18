// The page never runs inside a frame: another site could show it under its own page
// and trick a click on its buttons. Stopped here, nothing of the page is shown.
if (window.top !== window.self) {
    throw new Error("The admin page cannot run inside a frame");
}

// Same origin as the page: the session cookie is sent to the admin routes only.
// nginx forwards /generator/api/ to rs-gen-server.
const ADMIN_URL = "/generator/api/v1/admin";

// Header required on every admin request
const ADMIN_HEADER = "X-Rs-Gen-Admin";

// Refresh period of the list while a model is on its way: learning, saving, testing or loading
const BUSY_REFRESH_MS = 1000;

/* =============================
   DOM References
============================= */
const loginCard = document.getElementById("login-card");
const loginForm = document.getElementById("login-form");
const password = document.getElementById("password");
const loginBtn = document.getElementById("login-btn");
const loginError = document.getElementById("login-error");

const adminPanel = document.getElementById("admin-panel");
const logoutBtn = document.getElementById("logout-btn");
const refreshBtn = document.getElementById("refresh-btn");
const updated = document.getElementById("updated");
const memory = document.getElementById("memory");
const models = document.getElementById("models");
const modelsError = document.getElementById("models-error");

const addHint = document.getElementById("add-hint");
const modelFile = document.getElementById("model-file");
const modelName = document.getElementById("model-name");
const checkBtn = document.getElementById("check-btn");
const addBtn = document.getElementById("add-btn");
const forceRow = document.getElementById("force-row");
const forceAdd = document.getElementById("force-add");
const report = document.getElementById("report");

const deleteModal = document.getElementById("delete-modal");
const deleteName = document.getElementById("delete-name");
const deleteConfirm = document.getElementById("delete-confirm");
const deleteError = document.getElementById("delete-error");
const deleteBtn = document.getElementById("delete-btn");
const cancelDeleteBtn = document.getElementById("cancel-delete-btn");

const passwordForm = document.getElementById("password-form");
const currentPassword = document.getElementById("current-password");
const newPassword = document.getElementById("new-password");
const newPasswordAgain = document.getElementById("new-password-again");
const passwordBtn = document.getElementById("password-btn");
const passwordMessage = document.getElementById("password-message");

// Shortest password accepted by the server
const MIN_PASSWORD_CHARS = 12;

let refreshTimer = null;
// File and name of the last check without problem: "Add the model" sends exactly them.
// `risky` when the model fits only by using swap: the addition then needs the force box.
let checked = null;
// Model waiting for the delete confirmation
let modelToDelete = null;

/* =============================
   Formatting
============================= */
const numberFormat = new Intl.NumberFormat("en-US");

function formatNumber(value) {
    return value === null || value === undefined ? "not loaded" : numberFormat.format(value);
}

// Every size is shown in megabytes, with two decimals, as in the messages of the server
function formatBytes(bytes) {
    if (bytes === null || bytes === undefined) {
        return "unknown";
    }
    if (bytes > 0 && bytes < 10000) {
        return "< 0.01 MB";
    }
    return `${(bytes / 1e6).toFixed(2)} MB`;
}

function formatDate(seconds) {
    if (seconds === null || seconds === undefined) {
        return "unknown";
    }
    const date = new Date(seconds * 1000);
    const pad = value => String(value).padStart(2, "0");
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

const STATUS_LABELS = {
    shown: "Shown",
    hidden: "Hidden",
    learning: "Learning",
    saving: "Saving the cache",
    testing: "Testing",
    loading: "Loading",
    failed: "Failed"
};

// Statuses of a model on its way: no action until it ends
const ON_THE_WAY = new Set(["learning", "saving", "testing", "loading"]);

// Label of the status of a model, with the share learned and the reason of a failed show
function statusLabel(model) {
    const label = STATUS_LABELS[model.status] || model.status;
    if (model.status === "learning" && model.progress !== null && model.progress !== undefined) {
        return `${label} ${Math.floor(model.progress * 100)} %`;
    }
    if (model.status === "hidden" && model.error) {
        return `${label}, show failed`;
    }
    return label;
}

/* =============================
   API
============================= */
class LoginRequired extends Error {}

// Calls an admin route. `body` is sent as JSON, or as is for a file.
async function api(method, path, body) {
    const options = { method, headers: { [ADMIN_HEADER]: "1" }, credentials: "same-origin" };
    if (body instanceof File) {
        options.headers["Content-Type"] = "application/octet-stream";
        options.body = body;
    } else if (body !== undefined) {
        options.headers["Content-Type"] = "application/json";
        options.body = JSON.stringify(body);
    }
    const response = await fetch(`${ADMIN_URL}${path}`, options);
    if (response.status === 401 && path !== "/login") {
        showLogin();
        throw new LoginRequired();
    }
    let data = null;
    try {
        data = await response.json();
    } catch {
        data = { error: `The server answered ${response.status}` };
    }
    return { status: response.status, ok: response.ok, data };
}

function showMessage(element, text) {
    element.textContent = text;
    element.classList.toggle("hidden", !text);
}

/* =============================
   Login
============================= */
function showLogin() {
    stopRefresh();
    adminPanel.classList.add("hidden");
    loginCard.classList.remove("hidden");
    password.focus();
}

async function showPanel() {
    loginCard.classList.add("hidden");
    adminPanel.classList.remove("hidden");
    await loadModels();
}

loginForm.addEventListener("submit", async event => {
    event.preventDefault();
    loginBtn.disabled = true;
    showMessage(loginError, "");
    try {
        const { ok, data } = await api("POST", "/login", { password: password.value });
        if (ok) {
            password.value = "";
            await showPanel();
        } else {
            showMessage(loginError, data.error);
        }
    } catch (error) {
        showMessage(loginError, `The server cannot be reached: ${error.message}`);
    } finally {
        loginBtn.disabled = false;
    }
});

logoutBtn.addEventListener("click", async () => {
    try {
        await api("POST", "/logout");
    } catch {
        // Logged out anyway
    }
    showLogin();
});

/* =============================
   Models
============================= */
function stopRefresh() {
    clearTimeout(refreshTimer);
    refreshTimer = null;
}

function renderMemory(data) {
    const figures = [
        ["Server process in RAM", formatBytes(data.memory.process_resident_bytes)],
        ["Server process in swap", formatBytes(data.memory.process_swap_bytes)],
        ["Available in RAM", formatBytes(data.available_bytes)],
        ["Available with swap", formatBytes(data.available_with_swap_bytes)],
        ["Service memory limit", data.memory.service_limit_bytes ? formatBytes(data.memory.service_limit_bytes) : "none"],
        ["Kept free", formatBytes(data.limits.memory_margin_bytes)]
    ];
    memory.replaceChildren(...figures.flatMap(([label, value]) => {
        const term = document.createElement("dt");
        term.textContent = label;
        const definition = document.createElement("dd");
        definition.textContent = value;
        return [term, definition];
    }));
    addHint.textContent = `One word per line, UTF-8, at most ${formatBytes(data.limits.max_upload_bytes)} `
        + `and ${data.limits.max_line_chars} characters per line. The check tells whether the server has the memory for it, `
        + "in RAM or only by using swap.";
}

function actionButton(label, title, onClick, className = "action-btn") {
    const button = document.createElement("button");
    button.className = `${className} small-btn`;
    button.textContent = label;
    button.title = title;
    button.addEventListener("click", onClick);
    return button;
}

// Column titles, shown above each figure when the table becomes cards on a phone
const COLUMN_LABELS = [...document.querySelectorAll("#models-table thead th")].map(th => th.textContent);

function renderModels(list) {
    models.replaceChildren(...list.map(model => {
        const row = document.createElement("tr");
        const cell = (text, className = "") => {
            const td = document.createElement("td");
            td.textContent = text;
            if (className) {
                td.className = className;
            }
            td.dataset.label = COLUMN_LABELS[row.children.length] || "";
            row.appendChild(td);
            return td;
        };

        cell(model.name, "model-name");
        const status = cell(statusLabel(model));
        status.classList.add("status", `status-${model.status}`);
        if (model.error) {
            status.title = model.error;
            status.classList.add("status-error");
        }
        cell(formatNumber(model.stats?.sentences), "number");
        cell(formatNumber(model.stats?.states), "number");
        cell(formatNumber(model.stats?.max_n), "number");
        cell(model.estimated_bytes === null ? "not loaded" : formatBytes(model.estimated_bytes), "number");
        cell(formatBytes(model.dat_bytes), "number");
        cell(formatBytes(model.bin_bytes), "number");
        cell(formatDate(model.modified));

        const actions = cell("", "actions");
        if (model.status === "shown") {
            actions.appendChild(actionButton("Hide", "Remove from the generator and free its memory; the files are kept", () => runAction(model.name, "hide")));
        } else if (model.status === "hidden") {
            actions.appendChild(actionButton("Show", "Load it again into the generator, if the memory allows it", () => runAction(model.name, "show")));
        }
        if (!ON_THE_WAY.has(model.status)) {
            actions.appendChild(actionButton("Delete", "Delete its files for good", () => openDelete(model.name), "danger-btn"));
        }
        return row;
    }));
}

async function loadModels() {
    stopRefresh();
    try {
        const { ok, data } = await api("GET", "/models");
        if (!ok) {
            showMessage(modelsError, data.error);
            return;
        }
        showMessage(modelsError, "");
        renderMemory(data);
        renderModels(data.models);
        updated.textContent = `Updated at ${new Date().toLocaleTimeString("en-GB")}`;
        if (data.models.some(model => ON_THE_WAY.has(model.status))) {
            refreshTimer = setTimeout(loadModels, BUSY_REFRESH_MS);
        }
    } catch (error) {
        if (!(error instanceof LoginRequired)) {
            showMessage(modelsError, `The server cannot be reached: ${error.message}`);
        }
    }
}

// `force` is sent once the admin accepted that the model fits only by using swap
async function runAction(name, action, force = false) {
    showMessage(modelsError, "");
    try {
        const query = force ? "?force=true" : "";
        const { status, ok, data } = await api("POST", `/models/${encodeURIComponent(name)}/${action}${query}`);
        if (!ok && status === 422 && data.forceable && !force
            && window.confirm(`${data.error}.\n\nShow '${name}' anyway?`)) {
            await runAction(name, action, true);
            return;
        }
        if (!ok) {
            showMessage(modelsError, data.error);
        }
    } catch (error) {
        if (!(error instanceof LoginRequired)) {
            showMessage(modelsError, `The server cannot be reached: ${error.message}`);
        }
    }
    await loadModels();
}

refreshBtn.addEventListener("click", async () => {
    refreshBtn.disabled = true;
    try {
        await loadModels();
    } finally {
        refreshBtn.disabled = false;
    }
});

/* =============================
   Delete
============================= */
function openDelete(name) {
    modelToDelete = name;
    deleteName.textContent = name;
    deleteConfirm.value = "";
    deleteBtn.disabled = true;
    showMessage(deleteError, "");
    deleteModal.classList.remove("hidden");
    deleteConfirm.focus();
}

function closeDelete() {
    modelToDelete = null;
    deleteModal.classList.add("hidden");
}

deleteConfirm.addEventListener("input", () => {
    deleteBtn.disabled = deleteConfirm.value !== modelToDelete;
});

cancelDeleteBtn.addEventListener("click", closeDelete);

deleteModal.addEventListener("mousedown", event => {
    if (event.target === deleteModal) {
        closeDelete();
    }
});

deleteBtn.addEventListener("click", async () => {
    deleteBtn.disabled = true;
    try {
        const { ok, data } = await api("DELETE", `/models/${encodeURIComponent(modelToDelete)}`, { confirm: deleteConfirm.value });
        if (!ok) {
            showMessage(deleteError, data.error);
            deleteBtn.disabled = false;
            return;
        }
        closeDelete();
        await loadModels();
    } catch (error) {
        if (!(error instanceof LoginRequired)) {
            showMessage(deleteError, `The server cannot be reached: ${error.message}`);
            deleteBtn.disabled = false;
        }
    }
});

/* =============================
   Add
============================= */
// Suggests a valid model name from the file name
function nameFromFile(fileName) {
    return fileName
        .replace(/\.[^.]*$/, "")
        .toLowerCase()
        .replace(/[^a-z0-9_-]+/g, "_")
        .replace(/^[_-]+/, "")
        .slice(0, 64);
}

function updateAddButtons() {
    const file = modelFile.files[0];
    checkBtn.disabled = !file || !modelName.value;
    // "Add the model" sends only the file and the name whose check passed;
    // a model that fits only by using swap needs the force box too
    const matches = checked !== null && checked.file === file && checked.name === modelName.value;
    forceRow.classList.toggle("hidden", !(matches && checked.risky));
    addBtn.disabled = !matches || (checked.risky && !forceAdd.checked);
}

function forgetCheck() {
    checked = null;
    forceAdd.checked = false;
}

modelFile.addEventListener("change", () => {
    const file = modelFile.files[0];
    if (file && !modelName.value) {
        modelName.value = nameFromFile(file.name);
    }
    report.classList.add("hidden");
    forgetCheck();
    updateAddButtons();
});

modelName.addEventListener("input", updateAddButtons);
forceAdd.addEventListener("change", updateAddButtons);

function renderReport(result, title) {
    const parts = [];
    const heading = document.createElement("h4");
    heading.textContent = title;
    parts.push(heading);

    const list = (items, className) => {
        if (!items || items.length === 0) {
            return;
        }
        const ul = document.createElement("ul");
        ul.className = className;
        for (const item of items) {
            const li = document.createElement("li");
            li.textContent = item;
            ul.appendChild(li);
        }
        parts.push(ul);
    };
    list(result.problems, "message message-error");
    list(result.risks, "message message-warning");
    list(result.warnings, "message message-warning");

    if (result.count) {
        const count = result.count;
        const figures = [
            ["Lines", formatNumber(count.lines)],
            ["Distinct words", formatNumber(count.stats.sentences)],
            ["States", count.complete ? formatNumber(count.stats.states) : `more than ${formatNumber(count.stats.states)}`],
            ["Max n", count.complete ? formatNumber(count.stats.max_n) : "unknown"],
            ["Memory once loaded (estimate)", formatBytes(result.estimated_bytes)],
            ["Memory while building (estimate)", formatBytes(result.needed_bytes)],
            ["Available in RAM", formatBytes(result.available_bytes)],
            ["Available with swap", formatBytes(result.available_with_swap_bytes)]
        ];
        const dl = document.createElement("dl");
        dl.className = "figures";
        for (const [label, value] of figures) {
            const term = document.createElement("dt");
            term.textContent = label;
            const definition = document.createElement("dd");
            definition.textContent = value;
            dl.append(term, definition);
        }
        parts.push(dl);
    }
    report.replaceChildren(...parts);
    report.classList.remove("hidden");
}

checkBtn.addEventListener("click", async () => {
    const file = modelFile.files[0];
    const name = modelName.value;
    checkBtn.disabled = true;
    addBtn.disabled = true;
    forgetCheck();
    try {
        const { status, data } = await api("POST", `/models/check?name=${encodeURIComponent(name)}`, file);
        if (status === 200) {
            const risky = data.problems.length === 0 && data.risks.length > 0;
            const title = data.ok ? "The model can be added"
                : risky ? "The model can be added only by using swap"
                : "The model cannot be added";
            renderReport(data, title);
            if (data.ok || risky) {
                checked = { file, name, risky };
            }
        } else {
            renderReport({ problems: [data.error] }, "The check failed");
        }
    } catch (error) {
        if (!(error instanceof LoginRequired)) {
            renderReport({ problems: [`The server cannot be reached: ${error.message}`] }, "The check failed");
        }
    }
    updateAddButtons();
});

addBtn.addEventListener("click", async () => {
    const { file, name, risky } = checked;
    const force = risky && forceAdd.checked;
    addBtn.disabled = true;
    checkBtn.disabled = true;
    try {
        const query = `name=${encodeURIComponent(name)}${force ? "&force=true" : ""}`;
        const { status, data } = await api("POST", `/models?${query}`, file);
        if (status === 202) {
            renderReport(data, force
                ? "The build started, using swap: the model becomes shown once it generates words"
                : "The build started: the model becomes shown once it generates words");
            modelFile.value = "";
            modelName.value = "";
            forgetCheck();
        } else if (status === 422) {
            renderReport(data, "The model cannot be added");
            forgetCheck();
        } else {
            renderReport({ problems: [data.error] }, "The addition failed");
        }
    } catch (error) {
        if (!(error instanceof LoginRequired)) {
            renderReport({ problems: [`The server cannot be reached: ${error.message}`] }, "The addition failed");
        }
    }
    updateAddButtons();
    await loadModels();
});

/* =============================
   Password
============================= */
function showPasswordMessage(text, success = false) {
    passwordMessage.classList.toggle("message-success", success);
    passwordMessage.classList.toggle("message-error", !success);
    showMessage(passwordMessage, text);
}

passwordForm.addEventListener("submit", async event => {
    event.preventDefault();
    if ([...newPassword.value].length < MIN_PASSWORD_CHARS) {
        showPasswordMessage(`The new password needs at least ${MIN_PASSWORD_CHARS} characters`);
        return;
    }
    if (newPassword.value !== newPasswordAgain.value) {
        showPasswordMessage("The two new passwords differ");
        return;
    }
    passwordBtn.disabled = true;
    showMessage(passwordMessage, "");
    try {
        const { ok, data } = await api("POST", "/password", { current: currentPassword.value, new: newPassword.value });
        if (ok) {
            passwordForm.reset();
            showPasswordMessage("The password is changed. The other sessions are closed.", true);
        } else {
            showPasswordMessage(data.error);
        }
    } catch (error) {
        if (!(error instanceof LoginRequired)) {
            showPasswordMessage(`The server cannot be reached: ${error.message}`);
        }
    } finally {
        passwordBtn.disabled = false;
    }
});

/* =============================
   Init
============================= */
async function init() {
    try {
        const { ok, data } = await api("GET", "/session");
        if (ok) {
            await showPanel();
            return;
        }
        // The admin is disabled, or the request was refused
        showMessage(loginError, data.error);
    } catch (error) {
        if (error instanceof LoginRequired) {
            return;
        }
        showMessage(loginError, `The server cannot be reached: ${error.message}`);
    }
    showLogin();
}

init();
