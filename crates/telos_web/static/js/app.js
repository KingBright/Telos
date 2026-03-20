// ==================== GLOBALS ====================
let currentTab = 'metrics';
let backendUptimeSeconds = 0;
const tabRanges = { metrics: 'day', knowledge: 'day', tokens: 'day', tools: 'day', workflows: 'day' };

// ==================== HELPERS ====================
function fmt(n) {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
    return String(n);
}
function rate(ok, total) {
    if (total === 0) return '—';
    return Math.round((ok / total) * 100) + '%';
}
function setText(id, v) { const el = document.getElementById(id); if (el) el.innerText = v; }
function escapeHtml(str) { const d = document.createElement('div'); d.appendChild(document.createTextNode(str)); return d.innerHTML; }

// ==================== TAB SWITCHING ====================
function switchTab(tabName) {
    currentTab = tabName;
    document.querySelectorAll('[id^="tab-"]').forEach(el => el.classList.add('hidden'));
    document.getElementById(`tab-${tabName}`).classList.remove('hidden');
    document.querySelectorAll('.tab-btn').forEach(btn => {
        if (btn.dataset.tab === tabName) btn.classList.add('active');
        else btn.classList.remove('active');
    });
    if (tabName === 'traces') fetchTraces();
    if (tabName === 'metrics') refreshMetricsTab();
    if (tabName === 'knowledge') refreshKnowledgeTab();
    if (tabName === 'tokens') refreshTokensTab();
    if (tabName === 'tools') refreshToolsTab();
    if (tabName === 'workflows') refreshWorkflowsTab();
}

// ==================== TIME RANGE ====================
function onRangeChange(tab) {
    const sel = document.getElementById(`${tab}RangeSelect`);
    if (sel) tabRanges[tab] = sel.value;
    if (tab === 'metrics') refreshMetricsTab();
    if (tab === 'knowledge') refreshKnowledgeTab();
    if (tab === 'tokens') refreshTokensTab();
    if (tab === 'tools') refreshToolsTab();
    if (tab === 'workflows') refreshWorkflowsTab();
}

// ==================== TAB 1: METRICS OVERVIEW ====================
async function refreshMetricsTab() {
    try {
        // Fetch both live metrics and historical aggregate
        const [liveRes, histRes] = await Promise.all([
            fetch('/api/v1/metrics'),
            fetch(`/api/v1/metrics/history?range=${tabRanges.metrics}`)
        ]);
        if (!liveRes.ok || !histRes.ok) return;
        const live = await liveRes.json();
        const hist = await histRes.json();
        const agg = hist.aggregate || {};

        // LLM Gateway
        const llmCalls = agg.total_llm_calls || 0;
        const llm429 = agg.total_429_errors || 0;
        const llmOther = agg.total_other_errors || 0;
        const llmErrors = llm429 + llmOther;
        const llmSuccess = llmCalls > 0 ? llmCalls - llmErrors : 0;
        const totalTokens = agg.total_tokens || 0;
        setText('m_llm_calls', fmt(llmCalls));
        setText('m_llm_success', fmt(llmSuccess));
        setText('m_llm_429', fmt(llm429));
        setText('m_llm_errors', fmt(llmOther));
        setText('m_llm_rate', rate(llmSuccess, llmCalls));
        setText('m_llm_avg_tok', llmCalls > 0 ? fmt(Math.round(totalTokens / llmCalls)) : '—');

        // Task Flow
        const taskOk = agg.task_success || 0;
        const taskFail = agg.task_failure || 0;
        const taskTotal = taskOk + taskFail;
        setText('m_task_total', fmt(taskTotal));
        setText('m_task_ok', fmt(taskOk));
        setText('m_task_fail', fmt(taskFail));
        setText('m_task_active', live.task_flow?.active_concurrent_tasks || 0);
        setText('m_task_rate', rate(taskOk, taskTotal));

        // Tool Execution
        const toolOk = agg.tool_exec_success || 0;
        const toolFail = agg.tool_exec_failure || 0;
        const toolTotal = toolOk + toolFail;
        setText('m_tool_ok', fmt(toolOk));
        setText('m_tool_fail', fmt(toolFail));
        setText('m_tool_rate', rate(toolOk, toolTotal));
        setText('m_tool_total', fmt(toolTotal));

        // Tool Creation & Iteration (from live metrics — history doesn't track these yet)
        const dt = live.dynamic_tooling || {};
        setText('m_tc_ok', fmt(dt.creation_success || 0));
        setText('m_tc_fail', fmt(dt.creation_failure || 0));
        setText('m_ti_ok', fmt(dt.iteration_success || 0));
        setText('m_ti_fail', fmt(dt.iteration_failure || 0));
        setText('m_tc_rate', rate(dt.creation_success || 0, (dt.creation_success || 0) + (dt.creation_failure || 0)));
        setText('m_ti_rate', rate(dt.iteration_success || 0, (dt.iteration_success || 0) + (dt.iteration_failure || 0)));

        // QA
        const qaPass = agg.qa_passes || 0;
        const qaFail = agg.qa_failures || 0;
        setText('m_qa_pass', fmt(qaPass));
        setText('m_qa_fail', fmt(qaFail));
        setText('m_qa_rate', rate(qaPass, qaPass + qaFail));

        // Agent Activity
        setText('m_proactive', fmt(live.agent?.proactive_interactions || 0));
        setText('m_sem_loop', fmt(live.task_flow?.semantic_loop_interventions || 0));

        // Workflow Reuse
        const wfStored = agg.workflow_stored || 0;
        const wfReused = agg.workflow_reused || 0;
        const wfReuseOk = agg.workflow_reuse_success || 0;
        setText('m_wf_stored', fmt(wfStored));
        setText('m_wf_reused', fmt(wfReused));
        setText('m_wf_rate', rate(wfReuseOk, wfReused));

        // Uptime
        backendUptimeSeconds = live.uptime_seconds || 0;
        updateUptimeClock();
    } catch (e) {
        console.error('Metrics tab error:', e);
    }
}

// ==================== TAB 2: KNOWLEDGE & TOOLS ====================
async function refreshKnowledgeTab() {
    try {
        const res = await fetch('/api/v1/metrics');
        if (!res.ok) return;
        const m = await res.json();

        // Memory Clusters
        const ep = m.memory_os?.episodic_nodes || 0;
        const sem = m.memory_os?.semantic_nodes || 0;
        const proc = m.memory_os?.procedural_nodes || 0;
        const total = ep + sem + proc;
        const maxBar = Math.max(total, 1);

        setText('k_episodic', `${ep} NODES`);
        setText('k_semantic', `${sem} NODES`);
        setText('k_procedural', `${proc} NODES`);
        setText('k_total_nodes', fmt(total));
        setText('k_distillations', fmt(m.memory_os?.distillation_count || 0));

        document.getElementById('bar_episodic').style.width = `${(ep / maxBar) * 100}%`;
        document.getElementById('bar_semantic').style.width = `${(sem / maxBar) * 100}%`;
        document.getElementById('bar_procedural').style.width = `${(proc / maxBar) * 100}%`;

        // Tool Assets
        const dt = m.dynamic_tooling || {};
        const toolsCreated = dt.creation_success || 0;
        const toolsIterated = dt.iteration_success || 0;
        const toolExec = (dt.execution_success || 0) + (dt.execution_failure || 0);
        const toolExecRate = rate(dt.execution_success || 0, toolExec);
        setText('k_tools_created', fmt(toolsCreated));
        setText('k_tools_iterated', fmt(toolsIterated));
        setText('k_tools_exec', fmt(toolExec));
        setText('k_tools_exec_rate', toolExecRate);
        setText('k_create_rate', rate(dt.creation_success || 0, (dt.creation_success || 0) + (dt.creation_failure || 0)));
        setText('k_iterate_rate', rate(dt.iteration_success || 0, (dt.iteration_success || 0) + (dt.iteration_failure || 0)));
    } catch (e) {
        console.error('Knowledge tab error:', e);
    }
}

// ==================== TAB 3: TOKEN ANALYTICS ====================
let taskCurrentPage = 0;
let allTaskData = [];
const TASKS_PER_PAGE = 10;

async function refreshTokensTab() {
    const range = tabRanges.tokens;
    try {
        const [histRes, agentRes, taskRes] = await Promise.all([
            fetch(`/api/v1/metrics/history?range=${range}`),
            fetch(`/api/v1/metrics/by-agent?range=${range}`),
            fetch(`/api/v1/metrics/by-task?range=${range}`)
        ]);
        if (!histRes.ok) return;
        const hist = await histRes.json();
        const agg = hist.aggregate || {};

        // Summary Cards
        const totalTokens = agg.total_tokens || 0;
        const totalCost = agg.total_cost || 0;
        const totalCalls = agg.total_llm_calls || 0;
        setText('t_total_tokens', fmt(totalTokens));
        setText('t_total_cost', '$' + totalCost.toFixed(4));
        setText('t_total_calls', fmt(totalCalls));
        setText('t_avg_tokens', totalCalls > 0 ? fmt(Math.round(totalTokens / totalCalls)) : '—');

        // Agent Table
        if (agentRes.ok) {
            const agentData = await agentRes.json();
            renderAgentTable(agentData.agents || []);
        }
        // Task Table (paginated)
        if (taskRes.ok) {
            const taskData = await taskRes.json();
            allTaskData = taskData.tasks || [];
            renderTaskPage();
        }
    } catch (e) {
        console.error('Tokens tab error:', e);
    }
}

function renderAgentTable(agents) {
    const tbody = document.getElementById('t_agent_table');
    if (!tbody) return;
    if (!agents.length) {
        tbody.innerHTML = '<tr><td colspan="5" class="text-center py-4 text-slate-600">No agent data yet</td></tr>';
        return;
    }
    let html = '';
    agents.forEach(a => {
        const avgTok = a.call_count > 0 ? Math.round(a.total_tokens / a.call_count) : 0;
        html += `<tr class="border-b border-glass-border/30 hover:bg-glass-border/10 transition-colors">
            <td class="text-left py-2 px-3 text-slate-200">${escapeHtml(a.agent_name)}</td>
            <td class="text-right py-2 px-3 text-slate-300">${a.call_count}</td>
            <td class="text-right py-2 px-3 text-slate-200 font-bold">${fmt(a.total_tokens)}</td>
            <td class="text-right py-2 px-3 text-amber-400">$${(a.total_cost || 0).toFixed(4)}</td>
            <td class="text-right py-2 px-3 text-slate-400">${fmt(avgTok)}</td>
        </tr>`;
    });
    tbody.innerHTML = html;
}

// ==================== TAB 4: TOOL INVENTORY ====================
async function refreshToolsTab() {
    const range = tabRanges.tools;
    try {
        const res = await fetch(`/api/v1/tools/summary?range=${range}`);
        if (!res.ok) return;
        const data = await res.json();
        const inv = data.inventory || {};
        const usage = data.usage_summary || {};

        // Summary Cards
        setText('tl_total', inv.total || 0);
        setText('tl_native', inv.native || 0);
        setText('tl_custom', inv.custom || 0);
        setText('tl_calls', fmt(usage.total_calls || 0));
        setText('tl_failures', fmt(usage.total_failure || 0));
        setText('tl_rate', usage.success_rate || '—');

        // Per-tool table
        const tools = data.tools || [];
        const tbody = document.getElementById('tl_tool_table');
        if (!tbody) return;
        if (!tools.length) {
            tbody.innerHTML = '<tr><td colspan="6" class="text-center py-4 text-slate-600">No tool data yet</td></tr>';
            return;
        }
        let html = '';
        tools.forEach(t => {
            const typeColor = t.tool_type === 'native' ? 'text-blue-400 bg-blue-400/10 border-blue-400/30' : 'text-amber-400 bg-amber-400/10 border-amber-400/30';
            const typeLabel = t.tool_type === 'native' ? 'BUILT-IN' : 'CUSTOM';
            const rateClass = t.failure_count > 0 ? 'text-amber-400' : 'text-primary';
            const version = t.version || '—';
            const iteration = t.iteration || '—';
            const lastUpdated = t.last_updated_ms ? new Date(t.last_updated_ms).toLocaleString() : '—';
            html += `<tr class="border-b border-glass-border/30 hover:bg-glass-border/10 transition-colors">
                <td class="text-left py-2 px-3 text-slate-200">${escapeHtml(t.tool_name)}</td>
                <td class="text-center py-2 px-3"><span class="text-[9px] font-mono px-2 py-0.5 rounded-full border ${typeColor}">${typeLabel}</span></td>
                <td class="text-right py-2 px-3 text-slate-400">${version}</td>
                <td class="text-right py-2 px-3 text-slate-400">${iteration}</td>
                <td class="text-right py-2 px-3 text-slate-300">${t.total_calls}</td>
                <td class="text-right py-2 px-3 text-primary">${t.success_count}</td>
                <td class="text-right py-2 px-3 text-red-400">${t.failure_count}</td>
                <td class="text-right py-2 px-3 ${rateClass} font-bold">${t.success_rate}</td>
                <td class="text-right py-2 px-3 text-slate-500 text-[10px]">${lastUpdated}</td>
            </tr>`;
        });
        tbody.innerHTML = html;
    } catch (e) {
        console.error('Tools tab error:', e);
    }
}

// ==================== TAB 5: WORKFLOWS ====================
async function refreshWorkflowsTab() {
    const range = tabRanges.workflows;
    try {
        const res = await fetch(`/api/v1/workflows/summary?range=${range}`);
        if (!res.ok) return;
        const data = await res.json();
        const summary = data.summary || {};

        // Summary Cards
        setText('wf_total', summary.total_stored || 0);
        setText('wf_reused', fmt(summary.total_reused || 0));
        setText('wf_success', fmt(summary.total_reuse_success || 0));
        setText('wf_failure', fmt(summary.total_reuse_failure || 0));
        setText('wf_rate', summary.reuse_success_rate || '—');

        // Per-workflow table
        const workflows = data.workflows || [];
        const tbody = document.getElementById('wf_table');
        if (!tbody) return;
        if (!workflows.length) {
            tbody.innerHTML = '<tr><td colspan="7" class="text-center py-4 text-slate-600">No workflow data yet</td></tr>';
            return;
        }
        let html = '';
        workflows.forEach(w => {
            const storedAt = w.stored_at_ms ? new Date(w.stored_at_ms).toLocaleString() : '—';
            const desc = w.description || '';
            const shortDesc = desc.length > 60 ? desc.substring(0, 60) + '...' : desc;
            const rateClass = w.reuse_failure > 0 ? 'text-amber-400' : 'text-primary';
            html += `<tr class="border-b border-glass-border/30 hover:bg-glass-border/10 transition-colors">
                <td class="text-left py-2 px-3 text-slate-200" title="${escapeHtml(w.workflow_id)}">${escapeHtml(w.workflow_id.substring(0, 12))}...</td>
                <td class="text-left py-2 px-3 text-slate-400" title="${escapeHtml(desc)}">${escapeHtml(shortDesc)}</td>
                <td class="text-right py-2 px-3 text-slate-500 text-[10px]">${storedAt}</td>
                <td class="text-right py-2 px-3 text-blue-400">${w.reuse_count}</td>
                <td class="text-right py-2 px-3 text-primary">${w.reuse_success}</td>
                <td class="text-right py-2 px-3 text-red-400">${w.reuse_failure}</td>
                <td class="text-right py-2 px-3 ${rateClass} font-bold">${w.success_rate}</td>
            </tr>`;
        });
        tbody.innerHTML = html;
    } catch (e) {
        console.error('Workflows tab error:', e);
    }
}

function renderTaskPage() {
    const totalPages = Math.max(1, Math.ceil(allTaskData.length / TASKS_PER_PAGE));
    if (taskCurrentPage >= totalPages) taskCurrentPage = totalPages - 1;
    if (taskCurrentPage < 0) taskCurrentPage = 0;
    
    const start = taskCurrentPage * TASKS_PER_PAGE;
    const pageData = allTaskData.slice(start, start + TASKS_PER_PAGE);
    
    // Update page info
    setText('t_task_page_info', allTaskData.length > 0
        ? `Page ${taskCurrentPage + 1}/${totalPages} (${allTaskData.length} tasks)`
        : '');
    
    renderTaskTable(pageData);
}

function taskPagePrev() { if (taskCurrentPage > 0) { taskCurrentPage--; renderTaskPage(); } }
function taskPageNext() {
    const totalPages = Math.ceil(allTaskData.length / TASKS_PER_PAGE);
    if (taskCurrentPage < totalPages - 1) { taskCurrentPage++; renderTaskPage(); }
}

function renderTaskTable(tasks) {
    const tbody = document.getElementById('t_task_table');
    if (!tbody) return;
    if (!tasks.length) {
        tbody.innerHTML = '<tr><td colspan="7" class="text-center py-4 text-slate-600">No task data yet</td></tr>';
        return;
    }
    let html = '';
    tasks.forEach(t => {
        const shortId = t.task_id.length > 12 ? t.task_id.substring(0, 8) + '…' : t.task_id;
        const timeStr = t.total_time_ms ? (t.total_time_ms / 1000).toFixed(1) + 's' : '—';
        const statusClass = t.fulfilled === true ? 'text-primary' : t.fulfilled === false ? 'text-red-400' : 'text-slate-500';
        const statusLabel = t.fulfilled === true ? '✓ PASS' : t.fulfilled === false ? '✗ FAIL' : '—';
        html += `<tr class="border-b border-glass-border/30 hover:bg-glass-border/10 transition-colors">
            <td class="text-left py-2 px-3 text-slate-300" title="${escapeHtml(t.task_id)}">${escapeHtml(shortId)}</td>
            <td class="text-right py-2 px-3 text-slate-300">${t.llm_calls || 0}</td>
            <td class="text-right py-2 px-3 text-slate-200 font-bold">${fmt(t.total_tokens || 0)}</td>
            <td class="text-right py-2 px-3 text-amber-400">$${(t.total_cost || 0).toFixed(4)}</td>
            <td class="text-right py-2 px-3 text-slate-300">${t.tools_called || 0}</td>
            <td class="text-right py-2 px-3 text-slate-400">${timeStr}</td>
            <td class="text-center py-2 px-3 ${statusClass} font-bold">${statusLabel}</td>
        </tr>`;
    });
    tbody.innerHTML = html;
}

// ==================== TAB 4: TRACES ====================
let traceAutoRefresh = true;
let traceRefreshTimer = null;
let expandedTraceIds = new Set();
let lastTraceData = null;

function toggleTraceAutoRefresh() {
    traceAutoRefresh = !traceAutoRefresh;
    setText('trace-auto-status', traceAutoRefresh ? 'ON' : 'OFF');
    if (traceAutoRefresh) startTraceRefresh();
    else stopTraceRefresh();
}

function startTraceRefresh() {
    if (traceRefreshTimer) clearInterval(traceRefreshTimer);
    traceRefreshTimer = setInterval(() => {
        if (currentTab === 'traces') fetchTraces();
    }, 5000);
}

function stopTraceRefresh() {
    if (traceRefreshTimer) { clearInterval(traceRefreshTimer); traceRefreshTimer = null; }
}

function toggleTrace(traceId) {
    const detailEl = document.getElementById(`trace-detail-${traceId}`);
    const chevronEl = document.getElementById(`trace-chevron-${traceId}`);
    if (!detailEl) return;
    if (expandedTraceIds.has(traceId)) {
        expandedTraceIds.delete(traceId);
        detailEl.classList.remove('open');
        if (chevronEl) chevronEl.innerText = 'chevron_right';
    } else {
        expandedTraceIds.add(traceId);
        detailEl.classList.add('open');
        if (chevronEl) chevronEl.innerText = 'expand_more';
    }
}

async function fetchTraces() {
    try {
        const response = await fetch('/api/v1/traces');
        if (!response.ok) throw new Error('Failed to fetch traces');
        const data = await response.json();
        renderTraces(data.traces || []);
    } catch (error) {
        console.error('Failed to fetch traces:', error);
        const container = document.getElementById('traces-container');
        if (container && !lastTraceData) {
            container.innerHTML = `
                <div class="text-center py-12">
                    <span class="material-symbols-outlined text-4xl text-slate-600 mb-2">cloud_off</span>
                    <p class="text-slate-500 font-mono text-sm">Unable to connect to daemon API</p>
                </div>`;
        }
    }
}

function renderTraces(traces) {
    const container = document.getElementById('traces-container');
    if (!container) return;
    const traceEvents = traces.filter(t => t.type === 'Trace');
    if (traceEvents.length === 0) {
        container.innerHTML = `<div class="text-center py-12"><span class="material-symbols-outlined text-4xl text-slate-600 mb-2">info</span><p class="text-slate-500 font-mono text-sm">No traces found in current session</p></div>`;
        return;
    }
    const newFp = JSON.stringify(traceEvents.map(t => t.task_id + t.node_id));
    const oldFp = lastTraceData ? JSON.stringify(lastTraceData.map(t => t.task_id + t.node_id)) : '';
    if (newFp === oldFp) return;
    lastTraceData = traceEvents;
    const savedExpanded = new Set(expandedTraceIds);
    const reversed = traceEvents.slice().reverse();
    let html = '';
    reversed.forEach((fb, i) => {
        const log = fb.trace;
        const traceId = `t-${i}`;
        const isExpanded = savedExpanded.has(traceId);
        let title = '', badge = '', detailsObj = {};
        if (log.LlmCall) {
            title = 'LLM Call';
            badge = `<span class="trace-badge-llm text-[10px] font-mono px-2 py-0.5 rounded-full">${log.LlmCall.model || 'unknown'}</span>`;
            detailsObj = log.LlmCall;
        } else if (log.ToolCall) {
            title = `Tool: ${log.ToolCall.name}`;
            badge = `<span class="trace-badge-tool text-[10px] font-mono px-2 py-0.5 rounded-full">${log.ToolCall.name}</span>`;
            detailsObj = log.ToolCall;
        }
        html += `
        <div class="trace-card rounded-lg overflow-hidden">
            <div class="trace-header flex items-center justify-between px-4 py-3" onclick="toggleTrace('${traceId}')">
                <div class="flex items-center gap-3">
                    <span id="trace-chevron-${traceId}" class="material-symbols-outlined text-slate-500 text-sm">${isExpanded ? 'expand_more' : 'chevron_right'}</span>
                    <span class="text-[10px] font-mono text-slate-500">[${fb.node_id}]</span>
                    <span class="text-xs font-bold text-slate-200">${title}</span>
                    ${badge}
                </div>
                <span class="text-[10px] font-mono text-slate-600 truncate max-w-[300px]">Task: ${fb.task_id}</span>
            </div>
            <div id="trace-detail-${traceId}" class="trace-details ${isExpanded ? 'open' : ''}">
                <div class="px-4 pb-4">
                    <pre class="trace-json p-4 rounded-lg font-mono">${escapeHtml(JSON.stringify(detailsObj, null, 2))}</pre>
                </div>
            </div>
        </div>`;
    });
    container.innerHTML = html;
    expandedTraceIds = savedExpanded;
}

// ==================== UPTIME ====================
function updateUptimeClock() {
    const h = String(Math.floor(backendUptimeSeconds / 3600)).padStart(2, '0');
    const m = String(Math.floor((backendUptimeSeconds % 3600) / 60)).padStart(2, '0');
    const s = String(backendUptimeSeconds % 60).padStart(2, '0');
    setText('uptime_clock', `${h}:${m}:${s}`);
}

// ==================== STARTUP ====================
window.addEventListener('DOMContentLoaded', () => {
    // Initial data load
    refreshMetricsTab();
    
    // Auto-refresh active tab every 5 seconds
    setInterval(() => {
        if (currentTab === 'metrics') refreshMetricsTab();
        else if (currentTab === 'knowledge') refreshKnowledgeTab();
        else if (currentTab === 'tokens') refreshTokensTab();
        else if (currentTab === 'tools') refreshToolsTab();
        else if (currentTab === 'workflows') refreshWorkflowsTab();
    }, 5000);
    
    // Uptime tick
    setInterval(() => { backendUptimeSeconds++; updateUptimeClock(); }, 1000);
    
    // Trace auto-refresh
    startTraceRefresh();
    
    // WebSocket for Live Logs
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/api/v1/stream`;
    const ws = new WebSocket(wsUrl);
    
    ws.onmessage = (event) => {
        try {
            const data = JSON.parse(event.data);
            const logContainer = document.getElementById('live_logs');
            if (!logContainer) return;
            const timeStr = new Date().toLocaleTimeString('en-US', { hour12: false });
            let prefix = 'SYSTEM', message = '', color = 'text-slate-300';
            if (data.AgentState) { prefix = data.AgentState.agent_name.toUpperCase(); message = data.AgentState.state; color = 'text-blue-400'; }
            else if (data.Output) { prefix = 'OUTPUT'; message = 'Received structured response'; color = 'text-green-400'; }
            else if (data.Progress) { prefix = 'PROGRESS'; message = `Node [${data.Progress.info.node_id}] is ${data.Progress.info.status}`; }
            else if (data.TaskCompleted) { prefix = 'COMPLETE'; message = `Task ${data.TaskCompleted.task_id} finished`; color = 'text-yellow-400'; }
            else if (data.Error) { prefix = 'ERROR'; message = data.Error.detail; color = 'text-red-500'; }
            else if (typeof data === 'string') { message = data; }
            else { message = JSON.stringify(data).substring(0, 100) + '...'; }
            logContainer.insertAdjacentHTML('beforeend', `<div class="flex gap-4"><span class="text-slate-600">[${timeStr}]</span><span class="text-primary font-bold">${prefix}:</span><span class="${color}">${message}</span></div>`);
            logContainer.scrollTop = logContainer.scrollHeight;
            if (logContainer.children.length > 100) logContainer.removeChild(logContainer.firstElementChild);
        } catch (e) { console.error('WS error:', e); }
    };
    ws.onclose = () => {
        const lc = document.getElementById('live_logs');
        if (lc) lc.insertAdjacentHTML('beforeend', '<div class="text-red-500">[System] Connection dropped.</div>');
    };
});
