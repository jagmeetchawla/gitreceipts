// Filter the spine by color with checkboxes. Each color box toggles that
// color's visibility; "all" is a master that checks every color, and it
// reflects whether all colors are currently on.
(function () {
  var all = document.getElementById('f-all');
  var boxes = Array.prototype.slice.call(document.querySelectorAll('.fc'));
  function apply() {
    boxes.forEach(function (b) {
      document.body.classList.toggle('hide-' + b.value, !b.checked);
    });
    all.checked = boxes.every(function (b) { return b.checked; });
  }
  all.addEventListener('change', function () {
    boxes.forEach(function (b) { b.checked = all.checked; });
    apply();
  });
  boxes.forEach(function (b) { b.addEventListener('change', apply); });
  apply();
})();
