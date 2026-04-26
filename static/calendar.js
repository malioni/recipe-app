// ----------------------------------------------------------------
// State
// ----------------------------------------------------------------
const SLOTS = ["breakfast", "lunch", "dinner"];
const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

let weekStart = getMonday(new Date()); // current week's Monday
let allRecipes = []; // cached recipe list
let calendarData = {}; // { "YYYY-MM-DD": { breakfast: entry, ... } }
let cookedData = {}; // { "YYYY-MM-DD-recipeId": true }

// Pending slot for modal selection — declared explicitly to avoid
// implicit globals which break in strict mode.
let pendingDate = null;
let pendingSlot = null;

let pickModal = null;

// ----------------------------------------------------------------
// Date helpers
// ----------------------------------------------------------------
function getMonday(date) {
  const d = new Date(date);
  const day = d.getDay(); // 0 = Sun
  const diff = day === 0 ? -6 : 1 - day;
  d.setDate(d.getDate() + diff);
  d.setHours(0, 0, 0, 0);
  return d;
}

function addDays(date, n) {
  const d = new Date(date);
  d.setDate(d.getDate() + n);
  return d;
}

// Format a Date as YYYY-MM-DD using local time rather than UTC.
// Using toISOString() would return UTC and could produce the wrong
// date for users west of UTC (e.g. US timezones) late at night.
function toISO(date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(
    2,
    "0"
  )}-${String(date.getDate()).padStart(2, "0")}`;
}

function formatHeader(date) {
  const day = DAY_NAMES[date.getDay() === 0 ? 6 : date.getDay() - 1];
  return `${day}<br><span style="font-weight:400;color:#6c757d;">${date.getDate()}</span>`;
}

function isToday(date) {
  return toISO(date) === toISO(new Date());
}

// ----------------------------------------------------------------
// Navigation
// ----------------------------------------------------------------
function shiftWeek(delta) {
  weekStart = addDays(weekStart, delta * 7);
  loadWeek();
}

function goToToday() {
  weekStart = getMonday(new Date());
  loadWeek();
}

function updateWeekLabel() {
  const end = addDays(weekStart, 6);
  const fmt = (d) =>
    d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  document.getElementById("week-label").textContent = `${fmt(
    weekStart
  )} – ${fmt(end)}`;
}

// ----------------------------------------------------------------
// Load week data
// ----------------------------------------------------------------
async function loadWeek() {
  const navBtns = [
    document.getElementById("btn-prev-week"),
    document.getElementById("btn-today"),
    document.getElementById("btn-next-week"),
  ];
  navBtns.forEach((b) => (b.disabled = true));

  const grid = document.getElementById("calendar-grid");
  grid.innerHTML = `<div class="calendar-loading"><div class="spinner-border text-secondary" role="status"><span class="visually-hidden">Loading…</span></div></div>`;

  updateWeekLabel();
  const start = toISO(weekStart);
  const end = toISO(addDays(weekStart, 6));

  // Fetch meal plan and cooked log in parallel
  const [planRes, cookedRes] = await Promise.all([
    fetch(`/calendar/entries?start=${start}&end=${end}`),
    fetch(`/calendar/cooked?start=${start}&end=${end}`),
  ]).finally(() => navBtns.forEach((b) => (b.disabled = false)));

  calendarData = {};
  cookedData = {};

  if (planRes.ok) {
    const entries = await planRes.json();
    entries.forEach((e) => {
      if (!calendarData[e.date]) calendarData[e.date] = {};
      if (!calendarData[e.date][e.slot]) calendarData[e.date][e.slot] = [];
      calendarData[e.date][e.slot].push(e);
    });
  }

  if (cookedRes.ok) {
    const entries = await cookedRes.json();
    entries.forEach((e) => {
      cookedData[`${e.date}-${e.recipe_id}`] = true;
    });
  }

  renderGrid();
  prefillShoppingDates();
}

// ----------------------------------------------------------------
// Render grid
// ----------------------------------------------------------------
function renderGrid() {
  const grid = document.getElementById("calendar-grid");
  grid.innerHTML = "";

  const days = Array.from({ length: 7 }, (_, i) => addDays(weekStart, i));

  // Row 1: headers (empty corner + 7 day headers)
  appendCell(grid, "cell header", "");
  days.forEach((d) => {
    const cell = appendCell(
      grid,
      `cell header${isToday(d) ? " today-col" : ""}`,
      ""
    );
    cell.innerHTML = formatHeader(d);
  });

  // Rows 2-4: one row per slot
  SLOTS.forEach((slot) => {
    appendCell(
      grid,
      "cell slot-label",
      slot.charAt(0).toUpperCase() + slot.slice(1)
    );

    days.forEach((d) => {
      const dateStr = toISO(d);
      const cell = appendCell(
        grid,
        `cell${isToday(d) ? " today-col" : ""}`,
        ""
      );
      const entries = calendarData[dateStr]?.[slot] ?? [];

      entries.forEach((entry) => {
        const recipe = allRecipes.find((r) => r.id === entry.recipe_id);
        const name = recipe ? recipe.name : `Recipe #${entry.recipe_id}`;
        const cooked = !!cookedData[`${dateStr}-${entry.recipe_id}`];
        cell.appendChild(
          makeMealChip(name, entry.id, dateStr, entry.recipe_id, cooked, entry.portions ?? 1)
        );
      });

      // "+" button — hidden once the slot is full.
      // Must match MAX_ENTRIES_PER_SLOT in src/calendar_manager.rs.
      if (entries.length < 3) {
        const addBtn = document.createElement("button");
        addBtn.className = "add-meal-btn";
        addBtn.innerHTML = `<i class="bi bi-plus"></i> add`;
        addBtn.onclick = () => openPickModal(dateStr, slot);
        cell.appendChild(addBtn);
      }
    });
  });
}

function appendCell(grid, className, text) {
  const cell = document.createElement("div");
  cell.className = className;
  if (text) cell.textContent = text;
  grid.appendChild(cell);
  return cell;
}

function makeMealChip(name, entryId, date, recipeId, cooked, portions = 1) {
  const chip = document.createElement("div");
  chip.className = `meal-chip${cooked ? " cooked" : ""}`;

  // Name span — textContent prevents XSS from recipe names
  const nameSpan = document.createElement("span");
  nameSpan.className = "chip-name";
  nameSpan.title = portions > 1 ? `${name} ×${portions}` : name;
  nameSpan.textContent = name;

  if (portions > 1) {
    const badge = document.createElement("span");
    badge.className = "chip-portions";
    badge.textContent = `×${portions}`;
    nameSpan.appendChild(badge);
  }

  const actions = document.createElement("span");
  actions.className = "chip-actions";

  if (!cooked) {
    const cookBtn = document.createElement("button");
    cookBtn.className = "chip-btn";
    cookBtn.title = "Mark as cooked";
    cookBtn.innerHTML = `<i class="bi bi-check-lg"></i>`;
    cookBtn.addEventListener("click", function () {
      markCooked(date, recipeId, cookBtn);
    });
    actions.appendChild(cookBtn);
  }

  const removeBtn = document.createElement("button");
  removeBtn.className = "chip-btn text-danger";
  removeBtn.title = "Remove";
  removeBtn.innerHTML = `<i class="bi bi-x"></i>`;
  removeBtn.addEventListener("click", function () {
    removeMeal(entryId);
  });
  actions.appendChild(removeBtn);

  chip.appendChild(nameSpan);
  chip.appendChild(actions);
  return chip;
}

// ----------------------------------------------------------------
// Pick recipe modal
// ----------------------------------------------------------------
async function openPickModal(date, slot) {
  pendingDate = date;
  pendingSlot = slot;

  document.getElementById("modal-portions").value = 1;

  const list = document.getElementById("modal-recipe-list");
  list.innerHTML = `<div class="p-3 text-muted">Loading recipes…</div>`;
  pickModal.show();

  try {
    const res = await fetch("/recipes");
    if (res.ok) allRecipes = await res.json();
  } catch (err) {
    console.error("Could not load recipes:", err);
  }

  list.innerHTML = "";
  if (allRecipes.length === 0) {
    const msg = document.createElement("div");
    msg.className = "p-3 text-muted";
    msg.textContent = "No recipes found. ";
    const link = document.createElement("a");
    link.href = "/recipes/new";
    link.textContent = "Add one first.";
    msg.appendChild(link);
    list.appendChild(msg);
  } else {
    allRecipes.forEach((r) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "list-group-item list-group-item-action";
      btn.textContent = r.name;
      btn.onclick = () => {
        const portions = parseInt(document.getElementById("modal-portions").value, 10) || 1;
        planMeal(date, slot, r.id, portions);
      };
      list.appendChild(btn);
    });
  }
}

// ----------------------------------------------------------------
// API actions
// ----------------------------------------------------------------
async function planMeal(date, slot, recipeId, portions = 1) {
  pickModal.hide();
  try {
    const res = await fetch("/calendar/entries", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ date, slot, recipe_id: recipeId, portions }),
    });
    if (!res.ok) throw new Error(await res.text());
    await loadWeek();
  } catch (err) {
    console.error("Error planning meal:", err);
    alert("Could not save meal plan.");
  }
}

async function removeMeal(entryId) {
  try {
    const res = await fetch(`/calendar/entries?id=${entryId}`, {
      method: "DELETE",
    });
    if (!res.ok) throw new Error(await res.text());
    await loadWeek();
  } catch (err) {
    console.error("Error removing meal:", err);
    alert("Could not remove meal.");
  }
}

async function markCooked(date, recipeId, btn) {
  btn.disabled = true;
  try {
    const res = await fetch("/calendar/cooked", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ date, recipe_id: recipeId }),
    });
    if (!res.ok) throw new Error(await res.text());
    await loadWeek();
  } catch (err) {
    console.error("Error marking cooked:", err);
    btn.disabled = false;
    alert("Could not mark as cooked.");
  }
}

// ----------------------------------------------------------------
// Shopping list
// ----------------------------------------------------------------
function prefillShoppingDates() {
  document.getElementById("shop-start").value = toISO(weekStart);
  document.getElementById("shop-end").value = toISO(addDays(weekStart, 6));
}

async function loadShoppingList() {
  const start = document.getElementById("shop-start").value;
  const end = document.getElementById("shop-end").value;
  const emptyEl = document.getElementById("shop-empty");
  const errorEl = document.getElementById("shop-error");
  const listEl = document.getElementById("shop-list");

  emptyEl.classList.add("d-none");
  errorEl.classList.add("d-none");
  listEl.innerHTML = "";

  try {
    const res = await fetch(
      `/calendar/shopping-list?start=${start}&end=${end}`
    );
    if (!res.ok) throw new Error(await res.text());
    const ingredients = await res.json();

    if (ingredients.length === 0) {
      emptyEl.classList.remove("d-none");
      return;
    }

    ingredients.forEach((ing) => {
      const row = document.createElement("div");
      row.className = "ingredient-row";

      const nameSpan = document.createElement("span");
      nameSpan.textContent = ing.name;

      const qtySpan = document.createElement("span");
      qtySpan.className = "text-muted";
      const metricQty =
        ing.metric_quantity % 1 === 0
          ? ing.metric_quantity
          : parseFloat(ing.metric_quantity.toFixed(2));
      let qtyText = `${metricQty} ${ing.metric_unit}`;
      if (ing.imperial_quantity != null) {
        qtyText += ` (${ing.imperial_quantity} ${ing.imperial_unit})`;
      }
      qtySpan.textContent = qtyText;

      row.appendChild(nameSpan);
      row.appendChild(qtySpan);
      listEl.appendChild(row);
    });
  } catch (err) {
    console.error("Error loading shopping list:", err);
    errorEl.classList.remove("d-none");
  }
}

// ----------------------------------------------------------------
// Init
// ----------------------------------------------------------------
window.addEventListener("DOMContentLoaded", async () => {
  pickModal = new bootstrap.Modal(document.getElementById("pick-recipe-modal"));

  document
    .getElementById("btn-prev-week")
    .addEventListener("click", () => shiftWeek(-1));
  document.getElementById("btn-today").addEventListener("click", goToToday);
  document
    .getElementById("btn-next-week")
    .addEventListener("click", () => shiftWeek(1));
  document
    .getElementById("btn-generate-shopping")
    .addEventListener("click", loadShoppingList);

  await loadWeek();
  loadShoppingList();
});
