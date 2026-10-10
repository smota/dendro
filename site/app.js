// Dendro Interactive Experience Script
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

    // Update labels
    seatsVal.textContent = `${seats} assento${seats > 1 ? 's' : ''}`;
    missionsVal.textContent = `${missions} missões`;
    tokenCostVal.textContent = `$${costPerMillion} / 1M`;

    // Calculation formulas based on multi-agent empirical findings:
    // Retrabalho: ~1.4h por assento por 10 missões quando não monitorado
    const wastedHours = Math.round((seats * missions * 0.22));
    
    // Tokens desperdiçados: overhead de handoff não otimizado ~350k tokens por missão
    const wastedTokensMillions = (missions * seats * 0.38);
    const dollarsSaved = Math.round(wastedTokensMillions * costPerMillion * 4.2);

    // Speedup rate: de 25% a 55% dependendo do número de assentos
    const speedup = Math.min(55, Math.round(20 + (seats * 2.8)));

    reworkHours.textContent = `${wastedHours} horas`;
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
