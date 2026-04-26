// State — set on DOMContentLoaded
let editingId = null; // null = add mode, number = edit mode

function addIngredient(item = "", qty = "", unit = "") {
  const container = document.getElementById("ingredients-list");
  const row = document.createElement("div");
  row.className = "row g-2 mb-2 align-items-center";

  const colItem = document.createElement("div");
  colItem.className = "col";
  const inputItem = document.createElement("input");
  inputItem.type = "text";
  inputItem.className = "form-control";
  inputItem.placeholder = "Item";
  inputItem.value = item;
  colItem.appendChild(inputItem);

  const colQty = document.createElement("div");
  colQty.className = "col-3";
  const inputQty = document.createElement("input");
  inputQty.type = "number";
  inputQty.step = "any";
  inputQty.className = "form-control";
  inputQty.placeholder = "Qty";
  inputQty.value = qty;
  colQty.appendChild(inputQty);

  const colUnit = document.createElement("div");
  colUnit.className = "col-3";
  const inputUnit = document.createElement("input");
  inputUnit.type = "text";
  inputUnit.className = "form-control";
  inputUnit.placeholder = "Unit";
  inputUnit.value = unit;
  colUnit.appendChild(inputUnit);

  const colBtn = document.createElement("div");
  colBtn.className = "col-auto";
  const removeBtn = document.createElement("button");
  removeBtn.type = "button";
  removeBtn.className = "btn btn-sm btn-danger";
  removeBtn.textContent = "x";
  removeBtn.addEventListener("click", () => row.remove());
  colBtn.appendChild(removeBtn);

  row.appendChild(colItem);
  row.appendChild(colQty);
  row.appendChild(colUnit);
  row.appendChild(colBtn);
  container.appendChild(row);
}

function addStep(text = "") {
  const container = document.getElementById("steps-list");
  const row = document.createElement("div");
  row.className = "input-group mb-2";

  const input = document.createElement("input");
  input.type = "text";
  input.className = "form-control";
  input.placeholder = "Step description";
  input.value = text;

  const removeBtn = document.createElement("button");
  removeBtn.type = "button";
  removeBtn.className = "btn btn-danger";
  removeBtn.textContent = "x";
  removeBtn.addEventListener("click", () => row.remove());

  row.appendChild(input);
  row.appendChild(removeBtn);
  container.appendChild(row);
}

function populateForm(recipe) {
  document.getElementById("recipe-title").value = recipe.name;
  document.getElementById("recipe-source-url").value = recipe.source_url ?? "";
  recipe.ingredients.forEach((ing) => addIngredient(ing.name, ing.quantity, ing.unit));
  recipe.instructions.forEach((step) => addStep(step));
}

async function submitRecipe() {
  const title = document.getElementById("recipe-title").value.trim();
  if (!title) {
    alert("Please enter a recipe title.");
    return;
  }

  const ingredients = Array.from(
    document.querySelectorAll("#ingredients-list .row")
  ).map((row) => {
    const inputs = row.querySelectorAll("input");
    return {
      name: inputs[0].value,
      quantity: parseFloat(inputs[1].value) || 0,
      unit: inputs[2].value,
    };
  });

  const instructions = Array.from(document.querySelectorAll("#steps-list input"))
    .map((input) => input.value)
    .filter((v) => v.trim() !== "");

  const recipe = {
    name: title,
    source_url: document.getElementById("recipe-source-url").value.trim() || null,
    ingredients,
    instructions,
  };

  const isEditing = editingId !== null;
  const url = isEditing ? `/recipes/${editingId}` : "/recipes";
  const method = isEditing ? "PUT" : "POST";

  try {
    const response = await fetch(url, {
      method,
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(recipe),
    });

    if (response.ok) {
      window.location.href = isEditing ? `/#/recipes/${editingId}` : "/";
    } else {
      alert(`Failed to ${isEditing ? "update" : "add"} recipe`);
    }
  } catch (err) {
    console.error("Error submitting recipe:", err);
    alert("Error submitting recipe");
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  document.getElementById("add-ingredient-btn").addEventListener("click", () => addIngredient());
  document.getElementById("add-step-btn").addEventListener("click", () => addStep());
  document.getElementById("submit-btn").addEventListener("click", submitRecipe);

  const params = new URLSearchParams(window.location.search);
  const idParam = params.get("id");

  if (idParam !== null) {
    editingId = parseInt(idParam, 10);
    document.getElementById("page-title").textContent = "Edit Recipe";
    document.title = "Edit Recipe";

    const backLink = document.getElementById("back-link");
    backLink.href = `/#/recipes/${editingId}`;
    backLink.style.removeProperty("display");

    try {
      const response = await fetch(`/recipes/${editingId}`);
      if (!response.ok) throw new Error("Not found");
      const recipe = await response.json();
      populateForm(recipe);
    } catch (err) {
      alert("Could not load recipe for editing.");
      window.location.href = "/";
    }
  } else {
    addIngredient();
    addStep();
  }
});
