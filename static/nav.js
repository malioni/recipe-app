(async () => {
  const container = document.getElementById('app-nav');
  if (!container) return;

  let me = null;
  try {
    const res = await fetch('/profile/me');
    if (res.ok) me = await res.json();
  } catch (_) {
    // If the endpoint is unreachable, render nav without admin link.
  }

  const nav = document.createElement('nav');
  nav.className = 'navbar navbar-expand-sm navbar-light bg-white border-bottom mb-3';

  const inner = document.createElement('div');
  inner.className = 'container-fluid';

  // Brand
  const brand = document.createElement('a');
  brand.className = 'navbar-brand fw-semibold';
  brand.href = '/';
  brand.textContent = 'Recipes';
  inner.appendChild(brand);

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
