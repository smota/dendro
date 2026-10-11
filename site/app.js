// Dendro Interactive Rework Scenario Modeler Script
document.addEventListener('DOMContentLoaded', () => {
  const missionsInput = document.getElementById('missionsInput');
  const tokensPerMissionInput = document.getElementById('tokensPerMissionInput');
  const reworkRateInput = document.getElementById('reworkRateInput');
  const tokenCostInput = document.getElementById('tokenCostInput');

  const missionsVal = document.getElementById('missionsVal');
  const tokensPerMissionVal = document.getElementById('tokensPerMissionVal');
  const reworkRateVal = document.getElementById('reworkRateVal');
  const tokenCostVal = document.getElementById('tokenCostVal');

  const reworkChainsVal = document.getElementById('reworkChainsVal');
  const reworkTokensVal = document.getElementById('reworkTokensVal');
  const reworkCostVal = document.getElementById('reworkCostVal');

  function calculateRework() {
    const missions = parseInt(missionsInput.value, 10);
    const tokensK = parseInt(tokensPerMissionInput.value, 10);
    const reworkPercent = parseInt(reworkRateInput.value, 10);
    const costPerMillion = parseInt(tokenCostInput.value, 10);

    // Update slider label displays
    missionsVal.textContent = `${missions} missions`;
    tokensPerMissionVal.textContent = `${tokensK}k tokens`;
    reworkRateVal.textContent = `${reworkPercent}%`;
    tokenCostVal.textContent = `$${costPerMillion} / 1M`;

    // Transparent calculation derived directly from operator inputs:
    const reworkRate = reworkPercent / 100;
    const reworkChains = Math.round(missions * reworkRate);
    const tokensPerMissionMillions = tokensK / 1000;
    const reworkTokensMillions = (missions * reworkRate * tokensPerMissionMillions);
    const reworkCost = Math.round(reworkTokensMillions * costPerMillion);

    reworkChainsVal.textContent = `${reworkChains} chain${reworkChains === 1 ? '' : 's'}`;
    reworkTokensVal.textContent = `${reworkTokensMillions.toFixed(1)}M tokens`;
    reworkCostVal.textContent = `$${reworkCost.toLocaleString()} USD`;
  }

  if (missionsInput && tokensPerMissionInput && reworkRateInput && tokenCostInput) {
    missionsInput.addEventListener('input', calculateRework);
    tokensPerMissionInput.addEventListener('input', calculateRework);
    reworkRateInput.addEventListener('input', calculateRework);
    tokenCostInput.addEventListener('input', calculateRework);
    calculateRework();
  }
});
