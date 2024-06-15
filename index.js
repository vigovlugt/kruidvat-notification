fetch("https://www.kruidvat.nl/kruidvat-espressobonen/p/5437468", {
}).then(r => r.text()).then(console.log).catch(console.error);