// ----------------------------------------------------------------
// Router
// ----------------------------------------------------------------
function route() {
  const hash = window.location.hash; // e.g. "#/recipes/3"
  const match = hash.match(/^#\/recipes\/(\d+)$/);
  if (match) {
    showRecipeView(parseInt(match[1], 10));
  } else {
    showListView();
  }
}

function showListView() {
  document.getElementById("view-list").classList.remove("d-none");
  document.getElementById("view-recipe").classList.add("d-none");
  loadRecipeList();
}

function showRecipeView(id) {
  document.getElementById("view-list").classList.add("d-none");
  document.getElementById("view-recipe").classList.remove("d-none");
  loadRecipe(id);
}

window.addEventListener("hashchange", route);
window.addEventListener("DOMContentLoaded", () => {
  if (!window.location.hash) window.location.hash = "#/recipes";
  route();
  document
    .getElementById("delete-button")
    .addEventListener("click", deleteRecipe);
});

// ----------------------------------------------------------------
// List view
// ----------------------------------------------------------------
let recipeListLoading = false;

async function loadRecipeList() {
  // Guard against concurrent calls — navigating quickly or multiple
  // hashchange events firing at once can cause duplicate renders.
  if (recipeListLoading) return;
  recipeListLoading = true;
  const grid = document.getElementById("recipes-grid");
  const errorEl = document.getElementById("list-error");
  const emptyEl = document.getElementById("empty-state");
  const countEl = document.getElementById("recipe-count");

  grid.innerHTML = "";
  errorEl.classList.add("d-none");
  emptyEl.classList.add("d-none");

  try {
    const response = await fetch("/recipes");
    if (!response.ok) throw new Error("Server error");
    const recipes = await response.json();

    if (recipes.length === 0) {
      emptyEl.classList.remove("d-none");
      countEl.textContent = "";
      recipeListLoading = false;
      return;
    }

    countEl.textContent = `${recipes.length} recipe${
      recipes.length !== 1 ? "s" : ""
    }`;

    // Build recipe cards using DOM APIs to prevent XSS — never use
    // innerHTML with server-supplied values like recipe names.
    recipes.forEach((recipe) => {
      const col = document.createElement("div");
      col.className = "col-sm-6 col-md-4 col-lg-3";

      const link = document.createElement("a");
      link.href = `#/recipes/${recipe.id}`;
      link.className = "card shadow-sm h-100 recipe-card";

      const placeholder = document.createElement("div");
      placeholder.className =
        "d-flex align-items-center justify-content-center bg-secondary-subtle";
      placeholder.style.height = "160px";
      const icon = document.createElement("i");
      icon.className = "bi bi-journal-text text-secondary";
      icon.style.fontSize = "2.5rem";
      placeholder.appendChild(icon);

      const body = document.createElement("div");
      body.className = "card-body";

      const title = document.createElement("h5");
      title.className = "card-title mb-1";
      title.textContent = recipe.name;

      const meta = document.createElement("small");
      meta.className = "text-muted";
      meta.textContent = `${recipe.ingredients.length} ingredient${
        recipe.ingredients.length !== 1 ? "s" : ""
      } · ${recipe.instructions.length} step${
        recipe.instructions.length !== 1 ? "s" : ""
      }`;

      body.appendChild(title);
      body.appendChild(meta);
      link.appendChild(placeholder);
      link.appendChild(body);
      col.appendChild(link);
      grid.appendChild(col);
    });
  } catch (err) {
    console.error("Error loading recipes:", err);
    errorEl.classList.remove("d-none");
  } finally {
    recipeListLoading = false;
  }
}

// ----------------------------------------------------------------
// Recipe view
// ----------------------------------------------------------------
let currentRecipeId = null;

async function loadRecipe(id) {
  currentRecipeId = null;
  const nameEl = document.getElementById("recipe-name");
  const errorEl = document.getElementById("recipe-error");
  const contentEl = document.getElementById("recipe-content");
  const deleteBtn = document.getElementById("delete-button");

  nameEl.textContent = "Loading...";
  document.getElementById("recipe-source").style.display = "none";
  errorEl.classList.add("d-none");
  contentEl.classList.remove("d-none");
  deleteBtn.style.display = "none";
  document.getElementById("edit-button").style.display = "none";

  try {
    const response = await fetch(`/recipes/${id}`);
    if (!response.ok) throw new Error("Not found");
    const recipe = await response.json();

    currentRecipeId = id;
    renderRecipe(recipe);
  } catch (err) {
    nameEl.textContent = "Recipe not found";
    contentEl.classList.add("d-none");
    errorEl.classList.remove("d-none");
  }
}

function renderRecipe(recipe) {
  document.getElementById("recipe-name").textContent = recipe.name;

  const sourceEl = document.getElementById("recipe-source");
  const sourceLinkEl = document.getElementById("recipe-source-link");
  if (recipe.source_url) {
    sourceLinkEl.href = recipe.source_url;
    sourceEl.style.display = "block";
  } else {
    sourceEl.style.display = "none";
  }

  const ingredientsList = document.getElementById("ingredients-list");
  ingredientsList.innerHTML = "";
  recipe.ingredients.forEach((ing) => {
    const li = document.createElement("li");
    li.textContent = `${ing.quantity} ${ing.unit} ${ing.name}`;
    ingredientsList.appendChild(li);
  });

  const instructionsList = document.getElementById("instructions-list");
  instructionsList.innerHTML = "";
  recipe.instructions.forEach((step) => {
    const li = document.createElement("li");
    li.textContent = step;
    instructionsList.appendChild(li);
  });

  document.getElementById("delete-button").style.display = "flex";
  const editBtn = document.getElementById("edit-button");
  editBtn.href = `/recipes/edit?id=${recipe.id}`;
  editBtn.style.display = "flex";
}

async function deleteRecipe() {
  if (!currentRecipeId) return;
  if (!confirm("Are you sure you want to delete this recipe?")) return;

  try {
    const response = await fetch(`/recipes/${currentRecipeId}/delete`, {
      method: "POST",
    });

    if (response.ok) {
      window.location.hash = "#/recipes";
    } else {
      const errText = await response.text();
      alert("Failed to delete recipe: " + errText);
    }
  } catch (err) {
    console.error("Error deleting recipe:", err);
    alert("Error deleting recipe");
  }
}
