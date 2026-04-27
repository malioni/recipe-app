(async () => {
  const container = document.getElementById('app-nav');
  if (!container) return;

  let me = null;
  try {
    const res = await fetch('/profile/me');
    if (res.ok) me = await res.json();
  } catch (err) {
    console.warn('nav: /profile/me unreachable, rendering without admin link', err);
  }

  const nav = document.createElement('nav');
  nav.className = 'navbar navbar-expand-sm navbar-light bg-white border-bottom mb-3';

  const inner = document.createElement('div');
  inner.className = 'container-fluid';

  // Left-side brand links
  const leftLinks = document.createElement('div');
  leftLinks.className = 'd-flex align-items-center gap-3';

  const brand = document.createElement('a');
  brand.className = 'navbar-brand fw-semibold mb-0';
  brand.href = '/';
  brand.textContent = 'Recipes';
  leftLinks.appendChild(brand);

  const calendarLink = document.createElement('a');
  calendarLink.href = '/calendar';
  calendarLink.className = 'navbar-brand fw-semibold mb-0';
  calendarLink.textContent = 'Meal Planner';
  leftLinks.appendChild(calendarLink);

  inner.appendChild(leftLinks);

  // Right-side links
  const links = document.createElement('div');
  links.className = 'd-flex align-items-center gap-2';

  if (me && me.is_admin) {
    const adminLink = document.createElement('a');
    adminLink.href = '/admin';
    adminLink.className = 'btn btn-outline-secondary btn-sm';
    adminLink.textContent = 'Admin';
    links.appendChild(adminLink);
  }

  const profileLink = document.createElement('a');
  profileLink.href = '/profile';
  profileLink.className = 'btn btn-outline-secondary btn-sm';
  profileLink.textContent = 'Profile';
  links.appendChild(profileLink);

  const form = document.createElement('form');
  form.method = 'POST';
  form.action = '/logout';
  form.style.margin = '0';
  const signOutBtn = document.createElement('button');
  signOutBtn.type = 'submit';
  signOutBtn.className = 'btn btn-outline-danger btn-sm';
  signOutBtn.textContent = 'Sign Out';
  form.appendChild(signOutBtn);
  links.appendChild(form);

  inner.appendChild(links);
  nav.appendChild(inner);
  container.appendChild(nav);
})();
