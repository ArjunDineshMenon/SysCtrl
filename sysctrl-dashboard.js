// ============================================================================
// SysCtrl Dashboard - Vanilla JS Dashboard Logic
// Embed this script block at the bottom of <body> in the HTML mockup.
// ============================================================================

(function () {
  'use strict';

  // ==========================================================================
  // STATE
  // ==========================================================================
  const state = {
    cpu: { usage: 0, temp: 0, freq: 0, cores: 0 },
    gpu: { usage: 0, memUsed: 0, memTotal: 0, name: '' },
    ram: { used: 0, total: 0, percent: 0 },
    fans: {
      cpu: { target: 60, rpm: 0, auto: false },
      gpu1: { target: 35, rpm: 0, auto: false },
      gpu2: { target: 35, rpm: 0, auto: false },
    },
    chart: { range: '1m', cpuHistory: [], gpuHistory: [] },
    sensorsActive: true,
  };

  // ==========================================================================
  // DOM REFERENCES (populated after DOMContentLoaded)
  // ==========================================================================
  const els = {};

  // ==========================================================================
  // INITIALIZATION
  // ==========================================================================
  function init() {
    cacheElements();
    bindEvents();
    startMockDataLoop(); // Replace with real Tauri event listeners later
    requestAnimationFrame(renderLoop);
  }

  function cacheElements() {
    // Stat cards
    els.cpuUsageVal = document.getElementById('cpu-usage-val');
    els.cpuUsageBar = document.getElementById('cpu-usage-bar');
    els.cpuTempVal = document.getElementById('cpu-temp-val');
    els.cpuTempBar = document.getElementById('cpu-temp-bar');
    els.gpuUsageVal = document.getElementById('gpu-usage-val');
    els.gpuUsageBar = document.getElementById('gpu-usage-bar');
    els.ramUsedVal = document.getElementById('ram-used-val');
    els.ramUsedBar = document.getElementById('ram-used-bar');

    // Chart
    els.chartCpuPolyline = document.getElementById('chart-cpu-polyline');
    els.chartGpuPolyline = document.getElementById('chart-gpu-polyline');
    els.chartTabs = document.querySelectorAll('#chart-tab-1m, #chart-tab-5m, #chart-tab-1h');

    // Fan controls
    els.fanCpuSlider = document.getElementById('fan-cpu-slider');
    els.fanCpuVal = document.getElementById('fan-cpu-val');
    els.fanCpuRpm = document.getElementById('fan-cpu-rpm');
    els.fanGpu1Slider = document.getElementById('fan-gpu1-slider');
    els.fanGpu1Val = document.getElementById('fan-gpu1-val');
    els.fanGpu1Rpm = document.getElementById('fan-gpu1-rpm');
    els.fanGpu2Slider = document.getElementById('fan-gpu2-slider');
    els.fanGpu2Val = document.getElementById('fan-gpu2-val');
    els.fanGpu2Rpm = document.getElementById('fan-gpu2-rpm');
    els.fanAutoBtn = document.getElementById('fan-auto-btn');
    els.fanMaxBtn = document.getElementById('fan-max-btn');

    // Status
    els.statusPill = document.getElementById('status-pill');
    els.statusDot = document.getElementById('status-dot');
    els.statusText = document.getElementById('status-text');
  }

  function bindEvents() {
    // Chart tab switching
    els.chartTabs.forEach((tab) => {
      tab.addEventListener('click', () => switchChartRange(tab.dataset.range));
    });

    // Fan sliders
    bindFanSlider(els.fanCpuSlider, els.fanCpuVal, 'cpu');
    bindFanSlider(els.fanGpu1Slider, els.fanGpu1Val, 'gpu1');
    bindFanSlider(els.fanGpu2Slider, els.fanGpu2Val, 'gpu2');

    // Fan buttons
    els.fanAutoBtn?.addEventListener('click', () => setFanMode('auto'));
    els.fanMaxBtn?.addEventListener('click', () => setFanMode('max'));

    // Tauri event listeners (replace mock loop)
    // window.__TAURI__.event.listen('sensors://update', handleSensorUpdate);
    // window.__TAURI__.event.listen('fans://update', handleFanUpdate);
  }

  function bindFanSlider(slider, valEl, key) {
    if (!slider || !valEl) return;
    slider.addEventListener('input', (e) => {
      const v = parseInt(e.target.value, 10);
      valEl.textContent = v + '%';
      state.fans[key].target = v;
      // TODO: invoke Tauri command: invoke('set_fan_target', { fan: key, target: v });
    });
  }

  // ==========================================================================
  // MOCK DATA LOOP (replace with Tauri event listeners)
  // ==========================================================================
  function startMockDataLoop() {
    setInterval(() => {
      // Simulate sensor readings
      state.cpu.usage = clamp(state.cpu.usage + rand(-3, 3), 0, 100);
      state.cpu.temp = clamp(state.cpu.temp + rand(-2, 2), 30, 95);
      state.cpu.freq = 3400 + Math.round(rand(-200, 200));
      state.cpu.cores = 8;

      state.gpu.usage = clamp(state.gpu.usage + rand(-2, 2), 0, 100);
      state.gpu.memUsed = 2048 + Math.round(rand(-200, 200));
      state.gpu.memTotal = 6144;
      state.gpu.name = 'RTX 3060';

      state.ram.used = 11.2 + rand(-0.3, 0.3);
      state.ram.total = 32;
      state.ram.percent = Math.round((state.ram.used / state.ram.total) * 100);

      // Fan RPM simulation (roughly proportional to target %)
      state.fans.cpu.rpm = Math.round(state.fans.cpu.target * 30 + rand(-100, 100));
      state.fans.gpu1.rpm = Math.round(state.fans.gpu1.target * 30 + rand(-80, 80));
      state.fans.gpu2.rpm = Math.round(state.fans.gpu2.target * 30 + rand(-80, 80));

      // Chart history
      pushHistory(state.chart.cpuHistory, state.cpu.usage);
      pushHistory(state.chart.gpuHistory, state.gpu.usage);
    }, 1000);
  }

  function pushHistory(arr, val) {
    arr.push(val);
    if (arr.length > 60) arr.shift(); // keep last 60 points
  }

  // ==========================================================================
  // RENDER LOOP (runs ~60fps, but only updates DOM when values change)
  // ==========================================================================
  let lastRendered = {};
  function renderLoop() {
    renderStats();
    renderChart();
    renderFans();
    renderStatus();
    lastRendered = { ...state };
    requestAnimationFrame(renderLoop);
  }

  function renderStats() {
    // CPU Usage
    setText(els.cpuUsageVal, state.cpu.usage.toFixed(0) + '%');
    setStyle(els.cpuUsageBar, 'width', state.cpu.usage + '%');

    // CPU Temp
    setText(els.cpuTempVal, state.cpu.temp.toFixed(0) + '°C');
    setStyle(els.cpuTempBar, 'width', state.cpu.temp + '%');
    // Color temp based on threshold
    els.cpuTempVal.style.color = state.cpu.temp >= 85 ? 'var(--text-danger)' : state.cpu.temp >= 70 ? 'var(--text-warning)' : '';

    // GPU Usage
    setText(els.gpuUsageVal, state.gpu.usage.toFixed(0) + '%');
    setStyle(els.gpuUsageBar, 'width', state.gpu.usage + '%');

    // RAM
    setText(els.ramUsedVal, state.ram.used.toFixed(1) + ' GB / ' + state.ram.total + ' GB · ' + state.ram.percent + '%');
    setStyle(els.ramUsedBar, 'width', state.ram.percent + '%');
  }

  function renderChart() {
    const cpuPts = historyToPoints(state.chart.cpuHistory, 300, 80, true);
    const gpuPts = historyToPoints(state.chart.gpuHistory, 300, 80, true);
    setAttr(els.chartCpuPolyline, 'points', cpuPts);
    setAttr(els.chartGpuPolyline, 'points', gpuPts);
  }

  function historyToPoints(history, width, height, inverted) {
    if (!history.length) return '0,' + height + ' ' + width + ',' + height;
    const stepX = width / Math.max(1, history.length - 1);
    return history.map((v, i) => {
      const x = i * stepX;
      const y = inverted ? height - (v / 100) * height : (v / 100) * height;
      return x.toFixed(1) + ',' + y.toFixed(1);
    }).join(' ');
  }

  function renderFans() {
    // CPU fan
    setText(els.fanCpuVal, state.fans.cpu.target + '%');
    setText(els.fanCpuRpm, state.fans.cpu.rpm + ' RPM');
    els.fanCpuSlider.value = state.fans.cpu.target;

    // GPU fan 1
    setText(els.fanGpu1Val, state.fans.gpu1.target + '%');
    setText(els.fanGpu1Rpm, state.fans.gpu1.rpm + ' RPM');
    els.fanGpu1Slider.value = state.fans.gpu1.target;

    // GPU fan 2
    setText(els.fanGpu2Val, state.fans.gpu2.target + '%');
    setText(els.fanGpu2Rpm, state.fans.gpu2.rpm + ' RPM');
    els.fanGpu2Slider.value = state.fans.gpu2.target;
  }

  function renderStatus() {
    const active = state.sensorsActive;
    els.statusDot.style.background = active ? 'var(--text-success)' : 'var(--text-danger)';
    setText(els.statusText, active ? 'All sensors active' : 'Sensors disconnected');
  }

  // ==========================================================================
  // CHART RANGE SWITCHING
  // ==========================================================================
  function switchChartRange(range) {
    state.chart.range = range;
    els.chartTabs.forEach((t) => t.classList.toggle('active', t.dataset.range === range));
    // TODO: fetch history for selected range via Tauri command
  }

  // ==========================================================================
  // FAN CONTROL ACTIONS
  // ==========================================================================
  function setFanMode(mode) {
    if (mode === 'auto') {
      state.fans.cpu.auto = true;
      state.fans.gpu1.auto = true;
      state.fans.gpu2.auto = true;
      // TODO: invoke('set_fan_mode', { fan: 'all', mode: 'auto' });
    } else if (mode === 'max') {
      state.fans.cpu.target = 100;
      state.fans.gpu1.target = 100;
      state.fans.gpu2.target = 100;
      // TODO: invoke('set_fan_target', { fan: 'all', target: 100 });
    }
  }

  // ==========================================================================
  // TAURI EVENT HANDLERS (stubs - replace mock loop with these)
  // ==========================================================================
  // function handleSensorUpdate(event) {
  //   const data = event.payload;
  //   state.cpu = data.cpu;
  //   state.gpu = data.gpu;
  //   state.ram = data.ram;
  //   pushHistory(state.chart.cpuHistory, data.cpu.usage);
  //   pushHistory(state.chart.gpuHistory, data.gpu.usage);
  // }
  //
  // function handleFanUpdate(event) {
  //   const data = event.payload;
  //   state.fans[data.fan] = data;
  // }

  // ==========================================================================
  // UTILITIES
  // ==========================================================================
  function setText(el, text) {
    if (el && el.textContent !== text) el.textContent = text;
  }
  function setStyle(el, prop, val) {
    if (el && el.style[prop] !== val) el.style[prop] = val;
  }
  function setAttr(el, attr, val) {
    if (el && el.getAttribute(attr) !== val) el.setAttribute(attr, val);
  }
  function clamp(v, min, max) { return Math.max(min, Math.max(max, v)); }
  function rand(min, max) { return Math.random() * (max - min) + min; }

  // ==========================================================================
  // BOOT
  // ==========================================================================
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();