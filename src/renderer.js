(() => {
  const ruffleStatus = document.getElementById("ruffle-status");
  const swfStatus = document.getElementById("swf-status");
  const container = document.getElementById("player");

  const swfPath = "../assets/game.swf";

  if (!window.RufflePlayer) {
    ruffleStatus.textContent = "Ruffle: not found (missing vendor/ruffle/ruffle.js)";
    swfStatus.textContent = "SWF: not loaded";
    return;
  }

  ruffleStatus.textContent = "Ruffle: loaded";

  const ruffle = window.RufflePlayer.newest();
  const player = ruffle.createPlayer();

  player.config = {
    autoplay: "on",
    backgroundColor: "#000000",
    letterbox: "on"
  };

  container.innerHTML = "";
  container.appendChild(player);

  player
    .load(swfPath)
    .then(() => {
      swfStatus.textContent = `SWF: loaded (${swfPath})`;
    })
    .catch((err) => {
      swfStatus.textContent = "SWF: failed to load";
      console.error("Failed to load SWF", err);
    });
})();
