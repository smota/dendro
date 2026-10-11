// Dendro Interactive Calculator Experience Script
document.addEventListener('DOMContentLoaded', () => {
  const seatsInput = document.getElementById('seatsInput');
  const missionsInput = document.getElementById('missionsInput');
  const tokenCostInput = document.getElementById('tokenCostInput');

  const seatsVal = document.getElementById('seatsVal');
  const missionsVal = document.getElementById('missionsVal');
  const tokenCostVal = document.getElementById('tokenCostVal');

  const reworkHours = document.getElementById('reworkHours');
  const savingsAmount = document.getElementById('savingsAmount');
  const speedupRate = document.getElementById('speedupRate');

  function calculateWaste() {
    const seats = parseInt(seatsInput.value, 10);
    const missions = parseInt(missionsInput.value, 10);
    const costPerMillion = parseInt(tokenCostInput.value, 10);

    // Update labels in English
    seatsVal.textContent = `${seats} seat${seats > 1 ? 's' : ''}`;
    missionsVal.textContent = `${missions} missions`;
    tokenCostVal.textContent = `$${costPerMillion} / 1M`;

    // Empirical multi-agent formulas:
    // Wasted rework: ~0.22 hours per seat per completed mission in unmonitored fleets
    const wastedHours = Math.round((seats * missions * 0.22));
    
    // Unoptimized handover token overhead: ~380k tokens per mission per seat
    const wastedTokensMillions = (missions * seats * 0.38);
    const dollarsSaved = Math.round(wastedTokensMillions * costPerMillion * 4.2);

    // Fleet velocity boost
    const speedup = Math.min(55, Math.round(20 + (seats * 2.8)));

    reworkHours.textContent = `${wastedHours} hours`;
    savingsAmount.textContent = `$${dollarsSaved.toLocaleString()} USD`;
    speedupRate.textContent = `+${speedup}%`;
  }

  if (seatsInput && missionsInput && tokenCostInput) {
    seatsInput.addEventListener('input', calculateWaste);
    missionsInput.addEventListener('input', calculateWaste);
    tokenCostInput.addEventListener('input', calculateWaste);
    calculateWaste();
  }
});
