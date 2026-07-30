const invoke = window.__TAURI__.core.invoke;

const elements = {
  setupView: document.querySelector("#setup-view"),
  scanningView: document.querySelector("#scanning-view"),
  resultsView: document.querySelector("#results-view"),
  emptyLocations: document.querySelector("#empty-locations"),
  locationList: document.querySelector("#location-list"),
  addLocations: document.querySelector("#add-locations"),
  runCensus: document.querySelector("#run-census"),
  cancelCensus: document.querySelector("#cancel-census"),
  startOver: document.querySelector("#start-over"),
  exportReport: document.querySelector("#export-report"),
  setupStatus: document.querySelector("#setup-status"),
  exportStatus: document.querySelector("#export-status"),
  errorBanner: document.querySelector("#error-banner"),
  errorMessage: document.querySelector("#error-message"),
  dismissError: document.querySelector("#dismiss-error"),
  resultStatus: document.querySelector("#result-status"),
  resultSummary: document.querySelector("#result-summary"),
  metricArtifacts: document.querySelector("#metric-artifacts"),
  metricLocations: document.querySelector("#metric-locations"),
  metricBytes: document.querySelector("#metric-bytes"),
  metricCloud: document.querySelector("#metric-cloud"),
  categoryList: document.querySelector("#category-list"),
  signalsList: document.querySelector("#signals-list"),
};

let locations = [];

const categoryLabels = {
  pdf: "PDF",
  word_processing: "Word processing",
  email: "Email containers",
  plain_text: "Plain text & HTML",
  image_or_scan: "Images & scans",
  spreadsheet: "Spreadsheets",
  presentation: "Presentations",
  archive: "Compressed archives",
  database: "Databases",
  apple_document: "Apple documents",
  icloud_placeholder: "iCloud placeholders",
  other: "Other formats",
};

function showError(error) {
  elements.errorMessage.textContent =
    typeof error === "string" ? error : "The operation could not be completed safely.";
  elements.errorBanner.hidden = false;
}

function hideError() {
  elements.errorBanner.hidden = true;
  elements.errorMessage.textContent = "";
}

function showView(name) {
  elements.setupView.hidden = name !== "setup";
  elements.scanningView.hidden = name !== "scanning";
  elements.resultsView.hidden = name !== "results";
}

function setControlsDisabled(disabled) {
  elements.addLocations.disabled = disabled;
  elements.runCensus.disabled = disabled || locations.length === 0;
  for (const button of elements.locationList.querySelectorAll("button")) {
    button.disabled = disabled;
  }
}

function renderLocations(nextLocations) {
  locations = nextLocations;
  elements.locationList.replaceChildren();
  elements.emptyLocations.hidden = locations.length > 0;

  for (const location of locations) {
    const item = document.createElement("li");
    item.className = "location-item";

    const copy = document.createElement("div");
    copy.className = "location-copy";
    const icon = document.createElement("span");
    icon.className = "location-icon";
    icon.setAttribute("aria-hidden", "true");
    icon.textContent = "⌂";
    const text = document.createElement("div");
    const title = document.createElement("strong");
    title.textContent = location.label;
    const detail = document.createElement("span");
    detail.textContent = "Read-only metadata authority";
    text.append(title, detail);
    copy.append(icon, text);

    const remove = document.createElement("button");
    remove.className = "remove-location";
    remove.type = "button";
    remove.setAttribute("aria-label", `Remove ${location.label}`);
    remove.textContent = "×";
    remove.addEventListener("click", () => removeLocation(location.id));
    item.append(copy, remove);
    elements.locationList.append(item);
  }

  elements.runCensus.disabled = locations.length === 0;
  elements.setupStatus.textContent =
    locations.length === 0
      ? "Choose at least one location to continue."
      : `${locations.length.toLocaleString()} location${
          locations.length === 1 ? "" : "s"
        } approved. Nothing has been scanned yet.`;
}

async function chooseLocations() {
  hideError();
  setControlsDisabled(true);
  try {
    renderLocations(await invoke("choose_archive_locations"));
  } catch (error) {
    showError(error);
  } finally {
    setControlsDisabled(false);
  }
}

async function removeLocation(locationId) {
  hideError();
  setControlsDisabled(true);
  try {
    renderLocations(await invoke("remove_archive_location", { locationId }));
  } catch (error) {
    showError(error);
  } finally {
    setControlsDisabled(false);
  }
}

function formatBytes(bytes) {
  if (bytes === 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function createSignal(label, value) {
  const row = document.createElement("div");
  const term = document.createElement("dt");
  term.textContent = label;
  const description = document.createElement("dd");
  description.textContent = Number(value).toLocaleString();
  row.append(term, description);
  return row;
}

function renderReport(report) {
  const status = report.status.replaceAll("_", " ");
  elements.resultStatus.textContent = status;
  elements.resultStatus.dataset.status = report.status;
  elements.resultSummary.textContent =
    report.status === "complete"
      ? "The approved locations were counted without reading document contents."
      : report.status === "cancelled"
        ? "The census was cancelled. Partial counts were discarded from export."
        : "The census stopped at a safety boundary. Review the aggregate signals.";

  elements.metricArtifacts.textContent = report.summary.artifacts.toLocaleString();
  elements.metricLocations.textContent = report.summary.approved_locations.toLocaleString();
  elements.metricBytes.textContent = formatBytes(report.summary.regular_file_bytes);
  elements.metricCloud.textContent = report.signals.icloud_placeholders.toLocaleString();

  elements.categoryList.replaceChildren();
  const categories = [...report.categories].sort((left, right) => right.artifacts - left.artifacts);
  const largest = Math.max(1, ...categories.map((category) => category.artifacts));
  for (const category of categories.slice(0, 10)) {
    const row = document.createElement("div");
    row.className = "category-row";
    const label = document.createElement("span");
    label.className = "category-label";
    label.textContent = categoryLabels[category.category] ?? category.category;
    const track = document.createElement("span");
    track.className = "category-track";
    const fill = document.createElement("span");
    fill.style.width = `${Math.max(2, (category.artifacts / largest) * 100)}%`;
    track.append(fill);
    const count = document.createElement("span");
    count.className = "category-count";
    count.textContent = category.artifacts.toLocaleString();
    row.append(label, track, count);
    elements.categoryList.append(row);
  }
  if (categories.length === 0) {
    const empty = document.createElement("p");
    empty.className = "status";
    empty.textContent = "No file or package artifacts were found.";
    elements.categoryList.append(empty);
  }

  elements.signalsList.replaceChildren(
    createSignal("Likely OCR candidates", report.categories.find((item) => item.category === "image_or_scan")?.artifacts ?? 0),
    createSignal("iCloud placeholders", report.signals.icloud_placeholders),
    createSignal("Apple packages", report.summary.packages),
    createSignal("Unreadable by mode", report.signals.permission_mode_unreadable),
    createSignal("Links skipped", report.signals.symlinks_skipped),
    createSignal("Metadata errors", report.signals.metadata_errors + report.signals.directory_errors),
  );

  elements.exportReport.disabled = report.status !== "complete" && report.status !== "partial";
  elements.exportStatus.textContent = "";
  showView("results");
}

async function runCensus() {
  hideError();
  showView("scanning");
  try {
    const report = await invoke("run_archive_census");
    if (report.status === "cancelled") {
      showView("setup");
      elements.setupStatus.textContent = "Census cancelled. No report was retained.";
      return;
    }
    renderReport(report);
  } catch (error) {
    showView("setup");
    showError(error);
  }
}

async function cancelCensus() {
  elements.cancelCensus.disabled = true;
  elements.cancelCensus.textContent = "Cancelling…";
  try {
    await invoke("cancel_archive_census");
  } catch (error) {
    showError(error);
  } finally {
    elements.cancelCensus.textContent = "Cancel census";
    elements.cancelCensus.disabled = false;
  }
}

async function exportReport() {
  hideError();
  elements.exportReport.disabled = true;
  elements.exportStatus.textContent = "Preparing report…";
  try {
    const saved = await invoke("export_archive_census");
    elements.exportStatus.textContent = saved ? "Aggregate report saved." : "";
  } catch (error) {
    elements.exportStatus.textContent = "";
    showError(error);
  } finally {
    elements.exportReport.disabled = false;
  }
}

function startOver() {
  hideError();
  showView("setup");
  elements.setupStatus.textContent = `${locations.length.toLocaleString()} approved location${
    locations.length === 1 ? "" : "s"
  }. Change locations or run again.`;
}

async function bootstrap() {
  try {
    const state = await invoke("archive_bootstrap");
    renderLocations(state.locations);
    showView(state.scanRunning ? "scanning" : "setup");
  } catch (error) {
    showError(error);
  }
}

elements.addLocations.addEventListener("click", chooseLocations);
elements.runCensus.addEventListener("click", runCensus);
elements.cancelCensus.addEventListener("click", cancelCensus);
elements.startOver.addEventListener("click", startOver);
elements.exportReport.addEventListener("click", exportReport);
elements.dismissError.addEventListener("click", hideError);

bootstrap();
