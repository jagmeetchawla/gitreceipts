document.getElementById('filter').addEventListener('change', function () {
  document.body.classList.remove('f-red', 'f-red-residue');
  if (this.value !== 'all') document.body.classList.add('f-' + this.value);
});
