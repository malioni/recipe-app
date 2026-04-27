const alertArea = document.getElementById('alert-area');

function showAlert(msg, type = 'success') {
  const wrapper = document.createElement('div');
  wrapper.className = `alert alert-${type} alert-dismissible fade show`;
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

document.getElementById('password-form').addEventListener('submit', async e => {
  e.preventDefault();
  const res = await fetch('/profile/password', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      current_password: document.getElementById('current-password').value,
      new_password: document.getElementById('new-password').value,
    }),
  });
  const data = await res.json();
  if (res.ok) {
    showAlert('Password updated successfully');
    e.target.reset();
  } else {
    showAlert(data.error || 'Failed to update password', 'danger');
  }
});
