const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let chart = null;
let currentChildren = [];
let deleteTarget = null;
let deleteMode = "delete";
let currentLabelToPath = {};
let currentScan = null;
let othersExpanded = false;

const btnBack = document.getElementById("btn-back");
const btnOpen = document.getElementById("btn-open");
const statusEl = document.getElementById("status");
const welcomeEl = document.getElementById("welcome");
const chartContainer = document.getElementById("chart-container");
const chartEl = document.getElementById("chart");
const sidebar = document.getElementById("sidebar");
const fileList = document.getElementById("file-list");
const sidebarHeader = document.getElementById("sidebar-header");
const deleteDialog = document.getElementById("delete-dialog");
const deleteMessage = document.getElementById("delete-message");

function formatSize(bytes) {
    const KB = 1024, MB = KB * 1024, GB = MB * 1024, TB = GB * 1024;
    if (bytes >= TB) return (bytes / TB).toFixed(2) + " TB";
    if (bytes >= GB) return (bytes / GB).toFixed(2) + " GB";
    if (bytes >= MB) return (bytes / MB).toFixed(2) + " MB";
    if (bytes >= KB) return (bytes / KB).toFixed(2) + " KB";
    return bytes + " B";
}

function applyDarkTheme(options) {
    options.backgroundColor = "transparent";

    if (options.title) {
        if (Array.isArray(options.title)) {
            options.title.forEach(t => t.textStyle = { color: "#eee", fontSize: 16 });
        } else {
            options.title.textStyle = { color: "#eee", fontSize: 16 };
        }
    }
    if (options.legend) {
        const applyLegendStyle = (leg) => {
            leg.textStyle = { color: "#ccc", fontSize: 12 };
            leg.type = "scroll";
            leg.pageTextStyle = { color: "#ccc" };
        };
        if (Array.isArray(options.legend)) {
            options.legend.forEach(applyLegendStyle);
        } else {
            applyLegendStyle(options.legend);
        }
    }
    if (options.tooltip) {
        options.tooltip.backgroundColor = "rgba(22, 33, 62, 0.95)";
        options.tooltip.borderColor = "#2a2a4a";
        options.tooltip.textStyle = { color: "#eee" };
    }
}

function showView(scan) {
    welcomeEl.style.display = "none";
    chartContainer.style.display = "block";
    sidebar.style.display = "flex";

    currentScan = scan;
    currentChildren = scan.children;
    currentLabelToPath = scan.chart_label_to_path || {};
    othersExpanded = false;

    renderChart(scan.chart_options, scan.total_size);
    renderSidebar(scan.children, false);
    updateBackButton();

    if (scan.scanning) {
        statusEl.textContent = "Scanning...";
    } else {
        statusEl.textContent = "";
    }
}

function renderChart(chartOptions, totalSize) {
    if (!chart) {
        chart = echarts.init(chartEl, "dark");
        window.addEventListener("resize", () => chart.resize());
    }

    if (!chartOptions || totalSize === 0) {
        chart.clear();
        chart.showLoading({
            text: "Computing sizes...",
            color: "#e94560",
            textColor: "#eee",
            maskColor: "rgba(26, 26, 46, 0.8)",
        });
        return;
    }

    chart.hideLoading();

    const options = chartOptions;
    applyDarkTheme(options);
    chart.setOption(options, true);

    chart.off("click");
    chart.on("click", (params) => {
        if (params.componentType === "series" && params.data && params.data.name) {
            const label = params.data.name;
            const path = currentLabelToPath[label];
            if (path === "__OTHER__") {
                expandOthers();
            } else if (path) {
                navigateInto(path);
            }
        }
    });
}

async function expandOthers() {
    if (!currentScan || !currentScan.other_children || currentScan.other_children.length === 0) return;
    try {
        const result = await invoke("expand_others", {
            items: currentScan.other_children,
            parentName: currentScan.parent_name,
        });
        othersExpanded = true;
        currentLabelToPath = result.chart_label_to_path || {};
        const totalSize = result.items.reduce((sum, c) => sum + c.size, 0);
        renderChart(result.chart_options, totalSize);
        renderSidebar(result.items, true);
        updateBackButton();
    } catch (e) {
        statusEl.textContent = "Error: " + e;
    }
}

function collapseOthers() {
    if (!currentScan) return;
    othersExpanded = false;
    currentLabelToPath = currentScan.chart_label_to_path || {};
    renderChart(currentScan.chart_options, currentScan.total_size);
    renderSidebar(currentScan.children, false);
    updateBackButton();
    statusEl.textContent = "";
}

function renderSidebar(items, isOtherView) {
    fileList.innerHTML = "";
    sidebarHeader.innerHTML = "";

    const heading = document.createElement("h3");
    if (isOtherView) {
        const backLink = document.createElement("a");
        backLink.href = "#";
        backLink.className = "others-back";
        backLink.textContent = "\u2190 Back to overview";
        backLink.addEventListener("click", (e) => {
            e.preventDefault();
            collapseOthers();
        });
        sidebarHeader.appendChild(backLink);
        heading.textContent = "Other Items";
    } else {
        heading.textContent = "Contents";
    }
    sidebarHeader.appendChild(heading);

    const sorted = [...items].sort((a, b) => b.size - a.size);

    for (const child of sorted) {
        const item = document.createElement("div");
        item.className = "file-item";

        const icon = document.createElement("span");
        icon.className = "icon";
        icon.textContent = child.is_dir ? "\u{1F4C2}" : "\u{1F4C4}";

        const info = document.createElement("div");
        info.className = "info";

        const nameEl = document.createElement("div");
        nameEl.className = "name" + (child.is_dir ? " clickable" : "");
        nameEl.textContent = child.name;
        if (child.is_dir) {
            nameEl.addEventListener("click", () => navigateInto(child.path));
        }

        const sizeEl = document.createElement("div");
        sizeEl.className = "size";
        if (child.size > 0) {
            sizeEl.textContent = formatSize(child.size);
        } else if (child.is_dir) {
            sizeEl.innerHTML = '<span class="scanning-label">scanning\u2026</span>';
        } else {
            sizeEl.textContent = "0 B";
        }
        sizeEl.setAttribute("data-path", child.path);

        info.appendChild(nameEl);
        info.appendChild(sizeEl);

        const actions = document.createElement("div");
        actions.className = "item-actions";

        if (child.is_dir) {
            const cleanBtn = document.createElement("button");
            cleanBtn.className = "clean-btn";
            cleanBtn.textContent = "\u{1F9F9}";
            cleanBtn.title = "Clean (empty folder contents)";
            cleanBtn.addEventListener("click", (e) => {
                e.stopPropagation();
                showCleanDialog(child);
            });
            actions.appendChild(cleanBtn);
        }

        const delBtn = document.createElement("button");
        delBtn.className = "delete-btn";
        delBtn.textContent = "\u{1F5D1}";
        delBtn.title = "Delete";
        delBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            showDeleteDialog(child);
        });
        actions.appendChild(delBtn);

        item.appendChild(icon);
        item.appendChild(info);
        item.appendChild(actions);
        fileList.appendChild(item);
    }
}

async function navigateInto(path) {
    try {
        const result = await invoke("navigate_into", { path });
        showView(result);
    } catch (e) {
        statusEl.textContent = "Error: " + e;
    }
}

async function navigateBack() {
    if (othersExpanded) {
        collapseOthers();
        return;
    }
    try {
        const result = await invoke("navigate_back");
        if (result) showView(result);
    } catch (e) {
        statusEl.textContent = "Error: " + e;
    }
}

async function updateBackButton() {
    if (othersExpanded) {
        btnBack.disabled = false;
        return;
    }
    try {
        const has = await invoke("has_back_history");
        btnBack.disabled = !has;
    } catch (_) {
        btnBack.disabled = true;
    }
}

function showDeleteDialog(child) {
    deleteTarget = child;
    deleteMode = "delete";
    deleteMessage.textContent = 'Are you sure you want to delete "' + child.name + '"?';
    document.getElementById("delete-warning").textContent = "This action cannot be undone.";
    document.getElementById("btn-confirm-delete").textContent = "Delete";
    deleteDialog.style.display = "block";
}

function showCleanDialog(child) {
    deleteTarget = child;
    deleteMode = "clean";
    deleteMessage.textContent = 'Clean all contents of "' + child.name + '"?';
    document.getElementById("delete-warning").textContent = "The folder will be kept but all files and subfolders inside will be permanently removed.";
    document.getElementById("btn-confirm-delete").textContent = "Clean";
    deleteDialog.style.display = "block";
}

function hideDeleteDialog() {
    deleteTarget = null;
    deleteDialog.style.display = "none";
}

async function confirmDelete() {
    if (!deleteTarget) return;
    const path = deleteTarget.path;
    const mode = deleteMode;
    hideDeleteDialog();
    try {
        const cmd = mode === "clean" ? "clean_folder" : "delete_entry";
        const msg = await invoke(cmd, { path });
        statusEl.textContent = msg;
        const result = await invoke("get_current_view");
        if (result) showView(result);
    } catch (e) {
        statusEl.textContent = "Error: " + e;
    }
}

async function refreshCurrentView() {
    if (othersExpanded) return;
    try {
        const result = await invoke("get_current_view");
        if (result) showView(result);
    } catch (_) {}
}

btnOpen.addEventListener("click", async () => {
    try {
        const result = await invoke("open_folder");
        showView(result);
    } catch (e) {
        if (e !== "No folder selected") {
            statusEl.textContent = "Error: " + e;
        }
    }
});

btnBack.addEventListener("click", navigateBack);

document.getElementById("btn-confirm-delete").addEventListener("click", confirmDelete);
document.getElementById("btn-cancel-delete").addEventListener("click", hideDeleteDialog);
document.getElementById("delete-overlay").addEventListener("click", hideDeleteDialog);

let refreshTimer = null;
listen("size-update", (_event) => {
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => {
        refreshTimer = null;
        refreshCurrentView();
    }, 200);
});
