const form = document.querySelector('form');
const email = document.getElementById('email');
const password = document.getElementById('password');
const show = document.getElementById('show');
const remember = document.getElementById('remember');
const submit = document.getElementById('submit');
const status = document.getElementById('status');
show.addEventListener('change', function() {
    password.type = show.checked ? 'text' : 'password';
});
form.addEventListener('submit', function(event) {
    event.preventDefault();
    if (submit.disabled) return;
    email.removeAttribute('aria-invalid');
    password.removeAttribute('aria-invalid');
    let separator = -1;
    for (let i = 0; i < email.value.length; i = i + 1) {
        if (email.value[i] === '@') separator = i;
    }
    if (separator < 1 || separator === email.value.length - 1) {
        status.className = 'error';
        status.textContent = 'Enter an invented email with an @ sign.';
        email.setAttribute('aria-invalid', 'true');
        email.focus();
        return;
    }
    if (password.value.length < 4) {
        status.className = 'error';
        status.textContent = 'Enter at least four invented password characters.';
        password.setAttribute('aria-invalid', 'true');
        password.focus();
        return;
    }
    submit.disabled = true;
    submit.textContent = 'Connecting demo account...';
    email.disabled = true;
    password.disabled = true;
    show.disabled = true;
    remember.disabled = true;
    password.value = '';
    status.className = 'loading';
    status.textContent = 'Simulating authorization. No network request.';
    fixtureAuthorize(function(success) {
        if (success) {
            document.getElementById('heading').textContent = 'Demo account connected';
            form.className = 'hidden';
            status.className = 'success';
            status.textContent = 'Simulation complete. No account, token, or password was saved.';
        } else {
            submit.disabled = false;
            submit.textContent = 'Simulate connection';
            email.disabled = false;
            password.disabled = false;
            show.disabled = false;
            remember.disabled = false;
            status.className = 'error';
            status.textContent = 'Simulated authorization failed. Try again with invented details.';
            password.focus();
        }
    });
});
document.getElementById('about').addEventListener('click', function(event) {
    event.preventDefault();
    status.textContent = 'This isolated fixture has no provider connection or credential storage.';
});

email.focus();
