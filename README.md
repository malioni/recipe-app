# recipe-app
Rust Web App for maintaining recipes. The goal of the application is to host recipes for an individual or family that can be used to create a meal plan for the week (or month). It serves as 1) synchronized place for family recipes, 2) a convenient tool to plan upcoming meals and 3) an assistant during grocery shopping to create the grocery list to the planned meals. Most importantly, it's a project to develop skills. In this case Rust will be used during development to increase skills in this programming language. The code should be modern, efficient and well-tested. The goal is not to finish the application as fast as possible, but to learn new concepts and features in the process.

## To-do List
List of tasks for the project. Feature is implemented when it is merged into the main branch.
### Backend
- [ ] Create basic server structure: web app can be accessed from the browser
- [ ] Create an initial storage solution
- [ ] Create more robust storage solution
- [ ] Support for managing recipes - adding, modifying, deleting
- [ ] Ingredients list for recipes
- [ ] Support for planning recipes - adding, modifying, deleting recipes to the calendar
- [ ] Creating grocery list for recipes planned within certain time period

### Frontend
- [ ] Create recipe page, diplaying recipe, and ingredients
- [ ] Create calendar page, displaying calendar and recipes planned in certain days
- [ ] Create grocery list

### Future Work
- [ ] Create user accounts, logins
- [ ] Security features
- [ ] Move server to more permanent location
- [ ] Expose server to Internet
- [ ] Favorites
- [ ] Recipe search
- [ ] Ingredient search
- [ ] Pantry inventory

## Work Guidelines
- Use Rust for backend
- Documentation is done in code using Rust's documentation comments on all public components
- Any additional documentation can be added to the components README
- Tests are required for all public functions / interfaces
- Frontend can be done in whatever way is simplest
- Third-party open source libraries are allowed, while keeping in mind that the goal is to learn
