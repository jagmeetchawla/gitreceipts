document.getElementById('filter').addEventListener('change', function () {
  document.body.classList.remove('f-red', 'f-amber', 'f-green', 'f-red-amber');
  if (this.value !== 'all') document.body.classList.add('f-' + this.value);
});
