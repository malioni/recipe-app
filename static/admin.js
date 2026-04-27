const alertArea = document.getElementById('alert-area');
let currentUserId = null;

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function showAlert(msg, type = 'success') {
  const wrapper = document.createElement('div');
  wrapper.className = `alert alert-${escapeHtml(type)} alert-dismissible fade show`;
  wrapper.setAttribute('role', 'alert');
  wrapper.textContent = msg;
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'btn-close';
  btn.setAttribute('data-bs-dismiss', 'alert');
  wrapper.appendChild(btn);
  alertArea.innerHTML = '';
  alertArea.appendChild(wrapper);
}

async function loadCurrentUser() {
  const res = await fetch('/profile/me');
  if (res.ok) {
    const me = await res.json();
    currentUserId = me.id;
  }
}

async function deleteUser(id) {
  const res = await fetch('/admin/users/' + id, { method: 'DELETE' });
  const data = await res.json();
  if (res.ok) {
    showAlert('User deleted');
    loadUsers();
  } else {
    showAlert(data.error || 'Failed to delete user', 'danger');
  }
}

async function loadUsers() {
  const res = await fetch('/admin/users');
  if (!res.ok) { showAlert('Failed to load users', 'danger'); return; }
  const users = await res.json();
  const tbody = document.getElementById('user-rows');
  tbody.innerHTML = '';
  if (users.length === 0) {
    tbody.innerHTML = '<tr><td colspan="5" class="text-center text-muted py-3">No users</td></tr>';
    return;
  }
  users.forEach(u => {
    const tr = document.createElement('tr');

    const tdId = document.createElement('td');
    tdId.textContent = u.id;

    const tdName = document.createElement('td');
    tdName.textContent = u.username;

    const tdAdmin = document.createElement('td');
    const badge = document.createElement('span');
    badge.className = u.is_admin ? 'badge bg-danger' : 'badge bg-secondary';
    badge.textContent = u.is_admin ? 'admin' : 'user';
    tdAdmin.appendChild(badge);

    const tdCreated = document.createElement('td');
    tdCreated.textContent = u.created_at;

    const tdAction = document.createElement('td');
    const delBtn = document.createElement('button');
    delBtn.className = 'btn btn-outline-danger btn-sm';
    delBtn.textContent = 'Delete';
    if (u.id === currentUserId) {
      delBtn.disabled = true;
      delBtn.title = 'Cannot delete your own account';
    } else {
      delBtn.addEventListener('click', () => deleteUser(u.id));
    }
    tdAction.appendChild(delBtn);

    tr.appendChild(tdId);
    tr.appendChild(tdName);
    tr.appendChild(tdAdmin);
    tr.appendChild(tdCreated);
    tr.appendChild(tdAction);
    tbody.appendChild(tr);
  });
}

document.getElementById('create-form').addEventListener('submit', async e => {
  e.preventDefault();
  const res = await fetch('/admin/users', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      username: document.getElementById('new-username').value,
      password: document.getElementById('new-password').value,
    }),
  });
  const data = await res.json();
  if (res.ok) {
    showAlert('User created');
    e.target.reset();
    loadUsers();
  } else {
    showAlert(data.error || 'Failed to create user', 'danger');
  }
});

document.getElementById('password-form').addEventListener('submit', async e => {
  e.preventDefault();
  const res = await fetch('/admin/users/password', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      target_user_id: parseInt(document.getElementById('target-user-id').value, 10),
      new_password: document.getElementById('change-password').value,
    }),
  });
  const data = await res.json();
  if (res.ok) {
    showAlert('Password updated');
    e.target.reset();
  } else {
    showAlert(data.error || 'Failed to change password', 'danger');
  }
});

loadCurrentUser().then(() => loadUsers());
