'use strict';
// All values are operator assumptions. No network calls or hidden benchmark constants.
function modelScenario({seats,missions,tokensK,reworkPercent,costPerMillion,hours,reduction}) {
  const returned = missions * reworkPercent / 100;
  const tokenMillions = returned * tokensK / 1000;
  return {returned,tokenMillions,cost:tokenMillions*costPerMillion,
    hours:returned*hours*reduction/100,capacity:reworkPercent*reduction/100,
    missionsPerSeat:missions/seats};
}
document.addEventListener('DOMContentLoaded', () => {
  const fields={seats:'seatsInput',missions:'missionsInput',tokensK:'tokensPerMissionInput',reworkPercent:'reworkRateInput',costPerMillion:'tokenCostInput',hours:'hoursInput',reduction:'reductionInput'};
  const format=n=>n.toLocaleString(undefined,{maximumFractionDigits:2});
  const write=(id,text)=>{document.getElementById(id).textContent=text;};
  function update(){
    const input=Object.fromEntries(Object.entries(fields).map(([key,id])=>[key,Number(document.getElementById(id).value)]));
    const r=modelScenario(input);
    write('seatsVal',`${input.seats} actors`);write('missionsVal',`${input.missions} missions`);
    write('tokensPerMissionVal',`${input.tokensK}k tokens`);write('reworkRateVal',`${input.reworkPercent}%`);
    write('tokenCostVal',`$${input.costPerMillion} / 1M`);write('hoursVal',`${input.hours} hours`);write('reductionVal',`${input.reduction}%`);
    write('reworkChainsVal',`${format(r.returned)} expected returns`);write('reworkTokensVal',`${format(r.tokenMillions)}M tokens`);
    write('reworkCostVal',`$${r.cost.toLocaleString(undefined,{minimumFractionDigits:2,maximumFractionDigits:2})} USD`);
    write('hoursSavedVal',`${format(r.hours)} hours / month`);write('capacityVal',`${format(r.capacity)}%`);
    write('seatLoadVal',`${format(r.missionsPerSeat)} missions per actor / month, assuming an even workload.`);
  }
  if(Object.values(fields).every(id=>document.getElementById(id))){Object.values(fields).forEach(id=>document.getElementById(id).addEventListener('input',update));update();}
});
