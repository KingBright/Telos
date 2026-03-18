// Chart.js Configuration & Data Fetching
Chart.defaults.color = '#94a3b8'; // slate-400
Chart.defaults.font.family = "'Space Grotesk', sans-serif";
Chart.defaults.borderColor = 'rgba(51, 65, 85, 0.4)';
Chart.defaults.plugins.tooltip.backgroundColor = 'rgba(5, 5, 5, 0.9)';
Chart.defaults.plugins.tooltip.titleColor = '#f8fafc';
Chart.defaults.plugins.tooltip.bodyColor = '#e2e8f0';
Chart.defaults.plugins.tooltip.borderColor = 'rgba(0, 255, 65, 0.3)';
Chart.defaults.plugins.tooltip.borderWidth = 1;

let activityChart;
const MAX_HISTORY_POINTS = 30; // 30 seconds of history
const timeLabels = Array(MAX_HISTORY_POINTS).fill('');
const llmHistory = Array(MAX_HISTORY_POINTS).fill(0);
let lastLlmTotal = 0;
let isFirstFetch = true;

let backendUptimeSeconds = 0;

function initCharts() {
    const actCtx = document.getElementById('activityChart').getContext('2d');
    const gradientLlm = actCtx.createLinearGradient(0, 0, 0, 300);
    gradientLlm.addColorStop(0, 'rgba(0, 255, 65, 0.4)'); // neon green
    gradientLlm.addColorStop(1, 'rgba(0, 255, 65, 0.0)');

    activityChart = new Chart(actCtx, {
        type: 'line',
        data: {
            labels: timeLabels,
            datasets: [
                {
                    label: 'LLM Calls / sec',
                    data: llmHistory,
                    borderColor: '#00ff41',
                    backgroundColor: gradientLlm,
                    borderWidth: 2.5,
                    tension: 0.4,
                    fill: true,
                    pointRadius: 0,
                    pointHitRadius: 10,
                }
            ]
        },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            interaction: {
                mode: 'index',
                intersect: false,
            },
            plugins: {
                legend: { display: false }
            },
            scales: {
                x: { grid: { display: false }, ticks: { display: false } },
                y: { beginAtZero: true, suggestedMax: 10, grid: { borderDash: [4, 4] } }
            }
        }
    });
}

function updateRing(ringId, textId, pct, totalLen) {
    const circle = document.getElementById(ringId);
    const text = document.getElementById(textId);
    if (!circle || !text) return;
    
    // Smooth transition
    const offset = totalLen - (totalLen * (pct / 100));
    circle.style.strokeDashoffset = offset;
    text.innerText = `${Math.round(pct)}%`;
}

async function fetchMetrics() {
    try {
        const response = await fetch('/api/metrics');
        if (!response.ok) throw new Error('Network response was not ok');
        const data = await response.json();
        updateUI(data);
    } catch (error) {
        console.error('Failed to fetch metrics:', error);
    }
}

function updateUI(m) {
    // Memory
    document.getElementById('mem_episodic').innerText = `${m.memory_os.episodic_nodes} NODES`;
    document.getElementById('mem_semantic').innerText = `${m.memory_os.semantic_nodes} NODES`;
    document.getElementById('mem_procedural').innerText = `${m.memory_os.procedural_nodes} NODES`;
    
    const maxNodes = Math.max(m.memory_os.episodic_nodes + m.memory_os.semantic_nodes + m.memory_os.procedural_nodes, 100);
    document.getElementById('bar_episodic').style.width = `${(m.memory_os.episodic_nodes / maxNodes) * 100}%`;
    document.getElementById('bar_semantic').style.width = `${(m.memory_os.semantic_nodes / maxNodes) * 100}%`;
    document.getElementById('bar_procedural').style.width = `${(m.memory_os.procedural_nodes / maxNodes) * 100}%`;

    // Tool Outcomes (r=20, dasharray=125.6)
    const dt = m.dynamic_tooling;
    const toolTotal = dt.execution_success + dt.execution_failure;
    const toolPct = toolTotal > 0 ? (dt.execution_success / toolTotal) * 100 : 100;
    document.getElementById('val_tool_success').innerText = dt.execution_success;
    document.getElementById('val_tool_fails').innerText = dt.execution_failure;
    updateRing('ring_tool_pass', 'val_tool_pass_pct', toolPct, 125.6);

    // Task Outcomes (r=20, dasharray=125.6)
    const fl = m.task_flow;
    const taskTotal = fl.total_success + fl.total_failures;
    const taskPct = taskTotal > 0 ? (fl.total_success / taskTotal) * 100 : 100;
    document.getElementById('val_active_tasks').innerText = fl.active_concurrent_tasks;
    document.getElementById('val_task_success_count').innerText = fl.total_success;
    updateRing('ring_task_success', 'val_task_success_pct', taskPct, 125.6);

    // AI Proactivity
    document.getElementById('val_interactions').innerText = m.agent.proactive_interactions;

    // LLM Ingress
    document.getElementById('val_llm_reqs').innerText = m.llm.total_requests;
    document.getElementById('val_llm_429').innerText = m.llm.http_429_errors;
    document.getElementById('val_llm_errors').innerText = m.llm.other_api_errors;

    // QA Validation (r=34, dasharray=213.6)
    const qa = m.agent;
    const qaTotal = qa.qa_passes + qa.qa_failures;
    const qaPct = qaTotal > 0 ? (qa.qa_passes / qaTotal) * 100 : 100;
    document.getElementById('qa_pass_count').innerText = qa.qa_passes;
    document.getElementById('qa_fail_count').innerText = qa.qa_failures;
    
    if (qaPct < 80) {
        document.getElementById('qa_status_label').innerText = 'WARNING';
        document.getElementById('qa_status_label').className = 'text-[10px] font-mono text-orange-400';
    } else {
        document.getElementById('qa_status_label').innerText = 'SECURE';
        document.getElementById('qa_status_label').className = 'text-[10px] font-mono text-primary';
    }
    updateRing('ring_qa', 'val_qa_pct', qaPct, 213.6);

    // Apply Real Backend Uptime
    backendUptimeSeconds = m.uptime_seconds || 0;
    const h = String(Math.floor(backendUptimeSeconds / 3600)).padStart(2, '0');
    const u_m = String(Math.floor((backendUptimeSeconds % 3600) / 60)).padStart(2, '0');
    const s = String(backendUptimeSeconds % 60).padStart(2, '0');
    document.getElementById('uptime_clock').innerText = `${h}:${u_m}:${s}`;

    // Chart Time Series Update
    const now = new Date();
    const timeStr = `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`;
    
    let llmDelta = 0;
    if (!isFirstFetch) {
        llmDelta = Math.max(0, m.llm.total_requests - lastLlmTotal);
    }
    
    lastLlmTotal = m.llm.total_requests;
    isFirstFetch = false;

    timeLabels.push(timeStr);
    timeLabels.shift();
    
    llmHistory.push(llmDelta);
    llmHistory.shift();
    
    activityChart.update();
}

// Start sequence
window.addEventListener('DOMContentLoaded', () => {
    initCharts();
    fetchMetrics();
    // Poll every 1 second
    setInterval(fetchMetrics, 1000);
    
    // Connect to Daemon WebSocket for real-time Live Logs
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/api/v1/stream`;
    const ws = new WebSocket(wsUrl);
    
    ws.onmessage = (event) => {
        try {
            const data = JSON.parse(event.data);
            const logContainer = document.getElementById('live_logs');
            if (!logContainer) return;

            const timeStr = new Date().toLocaleTimeString('en-US', { hour12: false });
            let prefix = 'SYSTEM';
            let message = '';
            let color = 'text-slate-300';
            
            // Basic parsing of telos_hci::AgentFeedback enum
            if (data.AgentState) {
                prefix = data.AgentState.agent_name.toUpperCase();
                message = data.AgentState.state;
                color = 'text-blue-400';
            } else if (data.Output) {
                prefix = 'OUTPUT';
                message = 'Received structured response from Agent Graph';
                color = 'text-green-400';
            } else if (data.Progress) {
                prefix = 'PROGRESS';
                message = `Node [${data.Progress.info.node_id}] is ${data.Progress.info.status}`;
            } else if (data.TaskCompleted) {
                prefix = 'COMPLETE';
                message = `Task ID: ${data.TaskCompleted.task_id} gracefully finished.`;
                color = 'text-yellow-400';
            } else if (data.Error) {
                prefix = 'ERROR';
                message = data.Error.detail;
                color = 'text-red-500';
            } else if (typeof data === 'string') {
                message = data;
            } else {
                message = JSON.stringify(data).substring(0, 100) + '...';
            }
            
            const logHTML = `
                <div class="flex gap-4">
                    <span class="text-slate-600">[${timeStr}]</span>
                    <span class="text-primary font-bold">${prefix}:</span>
                    <span class="${color}">${message}</span>
                </div>
            `;
            logContainer.insertAdjacentHTML('beforeend', logHTML);
            
            // Auto-scroll
            logContainer.scrollTop = logContainer.scrollHeight;
            
            // Keep container max at ~100 logs
            if (logContainer.children.length > 100) {
                logContainer.removeChild(logContainer.firstElementChild);
            }
        } catch (e) {
            console.error('WebSocket parse error:', e);
        }
    };
    
    ws.onclose = () => {
        const logContainer = document.getElementById('live_logs');
        if (logContainer) {
            logContainer.insertAdjacentHTML('beforeend', '<div class="text-red-500">[System] Connection dropped.</div>');
        }
    };
});
