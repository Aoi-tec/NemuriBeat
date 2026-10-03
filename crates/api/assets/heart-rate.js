'use strict';
(() => {
  const el = id => document.getElementById(id);
  const params = new URLSearchParams(location.search);
  document.body.classList.toggle('transparent', params.get('transparent') === '1');
  document.body.classList.toggle('compact', params.get('compact') === '1');
  // A decorative waveform synthesized from BPM; the input contains no ECG samples.
  const canvas = el('trace');
  const ctx = canvas.getContext('2d');
  const motion = window.matchMedia('(prefers-reduced-motion: reduce)');
  const samples = new Float32Array(600); // Six seconds at 100 Hz, fixed memory.
  const knots = [[0,0],[.08,0],[.12,.12],[.16,0],[.18,-.18],[.20,1],[.23,-.35],[.27,0],[.36,.22],[.46,0],[1,0]];
  let head = 0, phase = 0, bpm = 0, frame = 0, previous = 0, pending = 0, painted = 0;
  function wave(p) {
    for (let i = 1; i < knots.length; i++) {
      if (p <= knots[i][0]) {
        const [x,y] = knots[i-1], [nextX,nextY] = knots[i];
        return y + (nextY-y) * (p-x) / (nextX-x);
      }
    }
    return 0;
  }
  function drawTrace() {
    if (!ctx) return;
    const width = canvas.clientWidth, height = canvas.clientHeight;
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    if (canvas.width !== Math.round(width*ratio) || canvas.height !== Math.round(height*ratio)) {
      canvas.width = Math.round(width*ratio); canvas.height = Math.round(height*ratio);
    }
    ctx.setTransform(ratio,0,0,ratio,0,0);
    ctx.clearRect(0,0,width,height);
    ctx.beginPath();
    for (let i = 0; i < samples.length; i++) {
      const x = i * width / (samples.length-1);
      const y = height*.56 - samples[(head+i)%samples.length]*height*.30;
      if (i === 0) ctx.moveTo(x,y); else ctx.lineTo(x,y);
    }
    ctx.strokeStyle = '#78c5e8'; ctx.lineWidth = 2; ctx.lineJoin = 'round';
    ctx.shadowColor = '#78c5e8'; ctx.shadowBlur = 6; ctx.stroke();
  }
  function animate(now) {
    frame = 0;
    if (!bpm || document.hidden || motion.matches || !ctx) return;
    pending += Math.min(Math.max(0, now-previous), 100);
    previous = now;
    while (pending >= 10) {
      phase = (phase + bpm/6000) % 1;
      samples[head] = wave(phase); head = (head+1)%samples.length;
      pending -= 10;
    }
    if (now-painted >= 1000/30) { drawTrace(); painted = now; }
    frame = requestAnimationFrame(animate);
  }
  function stopTrace() {
    cancelAnimationFrame(frame); frame = 0; bpm = 0;
    samples.fill(0); head = 0; phase = 0; pending = 0;
    drawTrace();
  }
  function startTrace(rate) {
    bpm = rate;
    if (!frame && !document.hidden && !motion.matches && ctx) {
      previous = performance.now(); painted = 0;
      frame = requestAnimationFrame(animate);
    }
  }
  document.addEventListener('visibilitychange', () => {
    const rate = bpm; stopTrace(); bpm = rate;
    // Resume only when the next fresh status arrives.
  });
  motion.addEventListener('change', () => {
    const rate = bpm; stopTrace(); startTrace(rate);
  });
  window.addEventListener('resize', drawTrace);
  drawTrace();
  let lastSuccess = 0;
  function unavailable(message) {
    stopTrace();
    document.body.classList.remove('live');
    el('bpm').textContent = '—';
    el('connection').textContent = message;
    el('sleep').textContent = '判定不能';
    el('age').textContent = '新しい心拍を受信すると復帰します';
  }
  function render(state) {
    if (!state.available || !Number.isInteger(state.heart_rate) || state.heart_rate < 1 || state.heart_rate > 300) {
      unavailable('心拍の受信待ち'); return;
    }
    document.body.classList.add('live');
    startTrace(state.heart_rate);
    el('bpm').textContent = String(state.heart_rate);
    el('source').textContent = state.source || 'SENSOR';
    el('connection').textContent = state.demo ? 'デモデータ' : '受信中';
    el('mode').textContent = state.mode === 'hr_rr' ? 'HR + RR' : 'HR ONLY';
    el('sleep').textContent = state.asleep === null ? '判定待ち' : state.estimated ? (state.asleep ? '擬似睡眠' : '擬似判定・覚醒') : (state.asleep ? '睡眠状態' : '覚醒状態');
    el('age').textContent = '更新 ' + (Math.max(0, state.age_ms) / 1000).toFixed(1) + ' 秒前';
  }
  async function poll() {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 2000);
    try {
      const response = await fetch('/api/v1/overlay/status', {cache:'no-store', signal:controller.signal});
      if (!response.ok) throw new Error('unavailable');
      const state = await response.json();
      render(state); lastSuccess = performance.now();
    } catch (_) { unavailable('NemuriBeatに接続できません'); }
    finally { clearTimeout(timeout); setTimeout(poll, 500); }
  }
  setInterval(() => { if (lastSuccess && performance.now() - lastSuccess > 2500) unavailable('データが途切れました'); }, 500);
  poll();
})();
