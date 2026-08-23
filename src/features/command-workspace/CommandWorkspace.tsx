/**
 * CMD-01 固定任务的统一 Command Workspace。
 *
 * 组件继承用户确认的 editorial-field-notes 视觉语法，只根据 Typed IPC 后端事实推进
 * Execution 生命周期。永久删除仍只是命令索引摘要，不在当前工作区获得任何真实动作。
 */
import { ArchiveBoxIcon } from "@phosphor-icons/react/dist/csr/BoxArrowDown";
import { ArrowBendRightDownIcon } from "@phosphor-icons/react/dist/csr/ArrowBendRightDown";
import { CalendarBlankIcon } from "@phosphor-icons/react/dist/csr/CalendarBlank";
import { ClockCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ClockCounterClockwise";
import { FileTextIcon } from "@phosphor-icons/react/dist/csr/FileText";
import { GearIcon } from "@phosphor-icons/react/dist/csr/Gear";
import { GitBranchIcon } from "@phosphor-icons/react/dist/csr/GitBranch";
import { HashIcon } from "@phosphor-icons/react/dist/csr/Hash";
import { MagnifyingGlassIcon } from "@phosphor-icons/react/dist/csr/MagnifyingGlass";
import { MinusIcon } from "@phosphor-icons/react/dist/csr/Minus";
import { PencilSimpleLineIcon } from "@phosphor-icons/react/dist/csr/PencilSimpleLine";
import { PlusIcon } from "@phosphor-icons/react/dist/csr/Plus";
import { ShieldCheckIcon } from "@phosphor-icons/react/dist/csr/ShieldCheck";
import { SquareIcon } from "@phosphor-icons/react/dist/csr/Square";
import { TerminalWindowIcon } from "@phosphor-icons/react/dist/csr/TerminalWindow";
import { TrashIcon } from "@phosphor-icons/react/dist/csr/Trash";
import { WarningIcon } from "@phosphor-icons/react/dist/csr/Warning";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { useMemo, useRef, useState } from "react";
import cmdboxIcon from "../../../src-tauri/icons/icon.png";
import {
  commandSummaries,
  commandSummaryCount,
  fixedExecutionSteps,
  type CommandSummary,
} from "./command-data";
import {
  createFixedExecutionGateway,
  type ApiError,
  type ExecutionStreamEvent,
  type FixedExecutionGateway,
} from "./execution-gateway";
import {
  appendExecutionOutput,
  createExecutionOutputBuffer,
  EXECUTION_OUTPUT_LIMIT_BYTES,
} from "./execution-output-buffer";
import {
  createDesktopWindowControls,
  type DesktopWindowControls,
} from "./desktop-window-controls";

/** 浏览器加载时解析一次真实桌面 Gateway；非 Tauri 环境保持 `null`。 */
const defaultGateway = createFixedExecutionGateway();

/** 浏览器加载时解析一次当前窗口控制；非 Tauri 环境保持 `null`。 */
const defaultWindowControls = createDesktopWindowControls();

/** 启动响应前最多缓存的事件数，限制无文本元数据事件的内存占用。 */
const PENDING_EVENT_LIMIT = 2048;

/** 预响应淘汰账本最多跟踪的不同 Execution ID 数量。 */
const PENDING_DROPPED_EXECUTION_LIMIT = 64;

/** 估算一个待认证事件占用的 UTF-8 负载字节数，并给元数据预留固定预算。 */
function pendingEventBytes(event: ExecutionStreamEvent): number {
  const metadataBudget = 128;
  if (event.event !== "output") {
    return metadataBudget;
  }
  return metadataBudget + event.data.fragments.reduce(
    (total, fragment) => total + new TextEncoder().encode(fragment.text).byteLength,
    0,
  );
}

/** 计算淘汰一个预响应 Output 事件时未保留的文本字节数。 */
function pendingOutputBytes(event: ExecutionStreamEvent): number {
  if (event.event !== "output") {
    return 0;
  }
  return event.data.fragments.reduce(
    (total, fragment) => total + new TextEncoder().encode(fragment.text).byteLength,
    0,
  );
}

/** Workspace 可由测试注入无副作用 Gateway。 */
export interface CommandWorkspaceProps {
  /** 固定任务 IPC；`null` 表示当前不是 Tauri 桌面宿主。 */
  gateway?: FixedExecutionGateway | null;
  /** 当前桌面主窗口控制；`null` 表示普通浏览器预览。 */
  windowControls?: DesktopWindowControls | null;
}

/** 前端根据后端事实派生的当前界面阶段。 */
type WorkspacePhase =
  | "ready"
  | "starting"
  | "running"
  | "cancelling"
  | "finished";

/** 唯一终态的用户可见摘要。 */
interface ExecutionResult {
  /** Rust Core 发布的终态类型。 */
  kind: "finished" | "cancelled" | "failed";
  /** 自然结束时存在的原始 Exit Code。 */
  exitCode?: number;
  /** Rust Core 记录的执行耗时。 */
  durationMs: number;
  /** 后端内部失败时的稳定公开说明。 */
  message?: string;
  /** Session 终态仍未随 Output 报告的丢弃字节数。 */
  droppedOutputBytes: number;
}

/** 为命令摘要选择统一图标注册表中的稳定图标。 */
function CommandSummaryIcon({ icon }: Pick<CommandSummary, "icon">) {
  const props = { "aria-hidden": true, size: 19, weight: "light" } as const;
  switch (icon) {
    case "terminal":
      return <TerminalWindowIcon {...props} />;
    case "delete":
      return <TrashIcon {...props} />;
    case "calendar":
      return <CalendarBlankIcon {...props} />;
    case "rename":
      return <FileTextIcon {...props} />;
    case "archive":
      return <ArchiveBoxIcon {...props} />;
    case "hash":
      return <HashIcon {...props} />;
    case "git":
      return <GitBranchIcon {...props} />;
  }
}

/** 渲染并控制 CMD-01 的真实固定任务工作区。 */
export function CommandWorkspace({
  gateway = defaultGateway,
  windowControls = defaultWindowControls,
}: CommandWorkspaceProps) {
  /** 当前命令索引搜索词。 */
  const [query, setQuery] = useState("");
  /** 当前由后端事实驱动的界面阶段。 */
  const [phase, setPhase] = useState<WorkspacePhase>("ready");
  /** 当前由启动响应确认的 Execution UUID。 */
  const [executionId, setExecutionId] = useState<string | null>(null);
  /** 当前任务的独立有界 Output Buffer。 */
  const [output, setOutput] = useState(createExecutionOutputBuffer);
  /** 当前唯一终态摘要。 */
  const [result, setResult] = useState<ExecutionResult | null>(null);
  /** 当前公开 IPC 错误。 */
  const [error, setError] = useState<ApiError | null>(null);
  /** 防止重复提交取消请求的局部请求状态。 */
  const [cancelRequestPending, setCancelRequestPending] = useState(false);
  /** 每次 Run 递增，用于隔离旧 Channel 回调。 */
  const runGeneration = useRef(0);
  /** 当前 Channel 已绑定的 Execution UUID。 */
  const expectedExecutionId = useRef<string | null>(null);
  /** 当前 Execution 已接受的最大事件级 sequence。 */
  const lastSequence = useRef(-1);
  /** 启动响应返回前暂存的 Channel 事件。 */
  const pendingEvents = useRef<ExecutionStreamEvent[]>([]);
  /** 当前预响应缓存的估算 UTF-8 负载字节数。 */
  const pendingEventsBytes = useRef(0);
  /** 按 Execution ID 隔离的预响应 Output 淘汰账本。 */
  const pendingDroppedOutputBytes = useRef(new Map<string, number>());
  /** 当前 Execution 是否已经接受唯一终态。 */
  const terminalAccepted = useRef(false);
  /** 与 React 状态同步的阶段快照，供异步响应防止状态倒退。 */
  const phaseSnapshot = useRef<WorkspacePhase>("ready");
  /** 当前仍有权修改取消请求状态的 generation。 */
  const cancelRequestGeneration = useRef<number | null>(null);

  /** 同步更新界面阶段和异步阶段快照。 */
  function transitionPhase(nextPhase: WorkspacePhase) {
    phaseSnapshot.current = nextPhase;
    setPhase(nextPhase);
  }

  /** 在有界分桶中记录一个被淘汰 Output 的文本和 Rust 已报告丢弃字节。 */
  function recordPendingOutputDrop(event: ExecutionStreamEvent) {
    if (event.event !== "output") {
      return;
    }
    const { executionId: droppedExecutionId, droppedBytesBefore } = event.data;
    const ledger = pendingDroppedOutputBytes.current;
    if (!ledger.has(droppedExecutionId) && ledger.size >= PENDING_DROPPED_EXECUTION_LIMIT) {
      const oldestExecutionId = ledger.keys().next().value as string | undefined;
      if (oldestExecutionId) {
        ledger.delete(oldestExecutionId);
      }
    }
    ledger.set(
      droppedExecutionId,
      (ledger.get(droppedExecutionId) ?? 0) + pendingOutputBytes(event) + droppedBytesBefore,
    );
  }

  /** 有界暂存响应前事件；超限时优先淘汰文本事件并保留生命周期事实。 */
  function queuePendingEvent(event: ExecutionStreamEvent) {
    pendingEvents.current.push(event);
    pendingEventsBytes.current += pendingEventBytes(event);
    while (
      pendingEventsBytes.current > EXECUTION_OUTPUT_LIMIT_BYTES ||
      pendingEvents.current.length > PENDING_EVENT_LIMIT
    ) {
      const outputIndex = pendingEvents.current.findIndex(
        (candidate) => candidate.event === "output",
      );
      const removalIndex = outputIndex >= 0 ? outputIndex : 0;
      const [removed] = pendingEvents.current.splice(removalIndex, 1);
      if (!removed) {
        break;
      }
      pendingEventsBytes.current -= pendingEventBytes(removed);
      recordPendingOutputDrop(removed);
    }
  }

  /** 按名称和说明过滤当前命令摘要。 */
  const filteredCommands = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");
    if (!normalizedQuery) {
      return commandSummaries;
    }
    return commandSummaries.filter((command) =>
      `${command.name} ${command.description}`
        .toLocaleLowerCase("zh-CN")
        .includes(normalizedQuery),
    );
  }, [query]);

  /** 接受当前 Channel 的严格有序后端事件并更新可观察状态。 */
  function acceptExecutionEvent(event: ExecutionStreamEvent, generation: number) {
    if (generation !== runGeneration.current) {
      return;
    }
    if (expectedExecutionId.current === null) {
      queuePendingEvent(event);
      return;
    }
    const { executionId: eventExecutionId, sequence } = event.data;
    if (
      eventExecutionId !== expectedExecutionId.current ||
      sequence <= lastSequence.current ||
      terminalAccepted.current
    ) {
      return;
    }
    lastSequence.current = sequence;

    switch (event.event) {
      case "started":
        transitionPhase("running");
        return;
      case "output":
        setOutput((current) =>
          appendExecutionOutput(
            current,
            event.data.sequence,
            event.data.fragments,
            event.data.droppedBytesBefore,
          ),
        );
        return;
      case "finished":
        terminalAccepted.current = true;
        setResult({
          kind: "finished",
          exitCode: event.data.exitCode,
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        transitionPhase("finished");
        return;
      case "cancelled":
        terminalAccepted.current = true;
        setResult({
          kind: "cancelled",
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        transitionPhase("finished");
        return;
      case "failed":
        terminalAccepted.current = true;
        setResult({
          kind: "failed",
          message: event.data.message,
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        transitionPhase("finished");
    }
  }

  /** 请求 Rust 启动固定验收任务；不传入任何脚本或进程参数。 */
  async function startExecution() {
    if (!gateway || phase === "starting" || phase === "running" || phase === "cancelling") {
      return;
    }
    const generation = runGeneration.current + 1;
    runGeneration.current = generation;
    expectedExecutionId.current = null;
    lastSequence.current = -1;
    pendingEvents.current = [];
    pendingEventsBytes.current = 0;
    pendingDroppedOutputBytes.current.clear();
    terminalAccepted.current = false;
    cancelRequestGeneration.current = null;
    setCancelRequestPending(false);
    setExecutionId(null);
    setOutput(createExecutionOutputBuffer());
    setResult(null);
    setError(null);
    transitionPhase("starting");
    try {
      const response = await gateway.startFixedExecution((event) => {
        acceptExecutionEvent(event, generation);
      });
      if (generation !== runGeneration.current) {
        return;
      }
      expectedExecutionId.current = response.executionId;
      setExecutionId(response.executionId);
      const bufferedEvents = pendingEvents.current;
      pendingEvents.current = [];
      pendingEventsBytes.current = 0;
      const authenticatedDroppedBytes =
        pendingDroppedOutputBytes.current.get(response.executionId) ?? 0;
      setOutput({
        ...createExecutionOutputBuffer(),
        droppedBytes: authenticatedDroppedBytes,
      });
      pendingDroppedOutputBytes.current.clear();
      for (const event of bufferedEvents) {
        acceptExecutionEvent(event, generation);
      }
    } catch (startError: unknown) {
      if (
        generation === runGeneration.current &&
        expectedExecutionId.current === null
      ) {
        pendingEvents.current = [];
        pendingEventsBytes.current = 0;
        pendingDroppedOutputBytes.current.clear();
        setError(startError as ApiError);
        transitionPhase("ready");
      }
    }
  }

  /** 请求 Rust 按当前 Execution UUID 终止整个 Job。 */
  async function cancelExecution() {
    if (!gateway || !executionId || phase !== "running" || cancelRequestPending) {
      return;
    }
    const generation = runGeneration.current;
    const targetExecutionId = executionId;
    cancelRequestGeneration.current = generation;
    setCancelRequestPending(true);
    setError(null);
    try {
      const response = await gateway.cancelExecution(targetExecutionId);
      if (
        generation === runGeneration.current &&
        targetExecutionId === expectedExecutionId.current &&
        !terminalAccepted.current &&
        phaseSnapshot.current === "running" &&
        response.state === "cancelling"
      ) {
        transitionPhase("cancelling");
      }
    } catch (cancelError: unknown) {
      if (
        generation === runGeneration.current &&
        targetExecutionId === expectedExecutionId.current &&
        !terminalAccepted.current
      ) {
        setError(cancelError as ApiError);
      }
    } finally {
      if (cancelRequestGeneration.current === generation) {
        cancelRequestGeneration.current = null;
        setCancelRequestPending(false);
      }
    }
  }

  /** 清空当前 UI Buffer，不影响 Rust Session、日志或终态。 */
  function clearOutput() {
    setOutput(createExecutionOutputBuffer());
  }

  /** 返回当前阶段的用户可读状态。 */
  function phaseLabel(): string {
    if (!gateway) return "需要桌面宿主";
    if (phase === "starting") return "正在建立执行";
    if (phase === "running") return "运行中";
    if (phase === "cancelling") return "正在终止进程树";
    if (phase === "finished") return "执行已结束";
    return "可以运行";
  }

  /** 执行窗口外壳动作并隔离其失败，避免污染 Execution 业务状态。 */
  function runWindowAction(action: () => Promise<void>) {
    void action().catch(() => undefined);
  }

  return (
    <main className="prototype-shell">
      <header className="window-bar" data-tauri-drag-region="deep">
        <div className="brand-lockup" data-tauri-drag-region="deep">
          <img src={cmdboxIcon} alt="" className="brand-mark" />
          <span className="brand-name">CmdBox</span>
          <span className="version-mark">v0.1.0</span>
        </div>
        <div className="window-caption" aria-label="窗口控制">
          <button type="button" className="caption-button" aria-label="最小化窗口" data-tauri-drag-region="false" disabled={!windowControls} onClick={() => windowControls && runWindowAction(windowControls.minimize)}>
            <MinusIcon size={16} weight="light" aria-hidden="true" />
          </button>
          <button type="button" className="caption-button" aria-label="最大化或还原窗口" data-tauri-drag-region="false" disabled={!windowControls} onClick={() => windowControls && runWindowAction(windowControls.toggleMaximize)}>
            <SquareIcon size={13} weight="light" aria-hidden="true" />
          </button>
          <button type="button" className="caption-button caption-button--close" aria-label="关闭窗口" data-tauri-drag-region="false" disabled={!windowControls} onClick={() => windowControls && runWindowAction(windowControls.close)}>
            <XIcon size={16} weight="light" aria-hidden="true" />
          </button>
        </div>
      </header>

      <div className="workspace-grid">
        <nav className="global-navigation" aria-label="主导航">
          <p className="rail-label">导航</p>
          <div className="global-navigation__items">
            <a className="rail-link rail-link--active" href="#command-workspace">
              <TerminalWindowIcon size={21} weight="light" aria-hidden="true" />
              <span>命令</span>
            </a>
            <a className="rail-link" href="#templates"><FileTextIcon size={21} weight="light" aria-hidden="true" /><span>模板</span></a>
            <a className="rail-link" href="#environments"><GitBranchIcon size={21} weight="light" aria-hidden="true" /><span>环境</span></a>
            <a className="rail-link" href="#schedules"><CalendarBlankIcon size={21} weight="light" aria-hidden="true" /><span>计划</span></a>
            <a className="rail-link" href="#history"><ClockCounterClockwiseIcon size={21} weight="light" aria-hidden="true" /><span>历史</span></a>
            <a className="rail-link" href="#settings"><GearIcon size={21} weight="light" aria-hidden="true" /><span>设置</span></a>
          </div>
          <div className="rail-footer">
            <img src={cmdboxIcon} alt="CmdBox" className="rail-footer__mark" />
            <p className="mono-label">CMD BOX</p>
            <p>在命令行确定性之间，建立可重复的一次性桥梁。</p>
            <div className="rail-footer__platform"><span>Windows 优先</span><span>本地执行 · 本地安全</span></div>
          </div>
        </nav>

        <aside className="command-index" aria-label="Command Block 索引">
          <div className="command-index__heading">
            <span>命令块索引</span>
            <button type="button" className="text-action" disabled><PlusIcon size={16} aria-hidden="true" />新建</button>
          </div>
          <label className="search-field">
            <span className="visually-hidden">搜索命令块</span>
            <MagnifyingGlassIcon size={18} aria-hidden="true" />
            <input type="search" placeholder="搜索命令块…" value={query} onChange={(event) => setQuery(event.currentTarget.value)} />
          </label>
          <p className="index-caption">全部命令块</p>
          <ul className="command-list">
            {filteredCommands.map((command) => (
              <li key={command.id}>
                <button type="button" className={`command-row${command.selected ? " command-row--selected" : ""}`} aria-current={command.selected ? "page" : undefined} disabled={!command.selected}>
                  <span className="command-row__icon"><CommandSummaryIcon icon={command.icon} /></span>
                  <span className="command-row__body"><strong>{command.name}</strong><small>{command.selected ? phaseLabel() : "尚未接入"} <span aria-hidden="true">·</span> {command.runner}</small></span>
                </button>
              </li>
            ))}
            {filteredCommands.length === 0 ? <li className="command-list__empty" role="status">没有匹配的命令块</li> : null}
          </ul>
          <p className="command-count">{query.trim() ? `显示 ${filteredCommands.length} / 共 ${commandSummaryCount} 个命令块` : `共 ${commandSummaryCount} 个命令块`}</p>
        </aside>

        <section className="command-workspace" id="command-workspace">
          <header className="workspace-heading">
            <p className="workspace-breadcrumb">命令工作区 <span>/</span> 执行链路验收</p>
            <h1>执行链路验收</h1>
            <p>运行 Rust Core 内置的无破坏任务，验证实时输出、自然结束和整棵进程树终止。</p>
          </header>

          <div className="workspace-content">
            <div className="evidence-column">
              <section className="runner-facts" aria-label="执行状态">
                <div><span>执行器</span><strong>Windows PowerShell 5.1</strong></div>
                <div><span>状态</span><strong className={gateway && phase !== "finished" ? "state-ready" : "state-stale"}>{phaseLabel()}</strong></div>
              </section>

              <section className="target-record" aria-labelledby="task-scope-title">
                <div className="section-heading-row"><h2 id="task-scope-title">固定任务范围</h2><span className="fixed-badge">无用户参数</span></div>
                <ol className="task-step-list">
                  {fixedExecutionSteps.map((step, index) => <li key={step}><span>{String(index + 1).padStart(2, "0")}</span><p>{step}</p></li>)}
                </ol>
              </section>

              <section className="preview-record execution-output-record" aria-labelledby="execution-output-title">
                <div className="preview-summary"><span>Execution</span><strong>{executionId ?? "尚未创建"}</strong></div>
                <div className="section-heading-row output-heading-row">
                  <h2 id="execution-output-title">实时输出</h2>
                  <button type="button" className="text-action" onClick={clearOutput} disabled={output.chunks.length === 0}>清空当前显示</button>
                </div>
                {output.droppedBytes > 0 ? <p className="output-truncation" role="status">早期实时输出已有 {output.droppedBytes} 字节未保留；外部任务未因此阻塞。</p> : null}
                <ol className="execution-output" aria-label="Execution 纯文本输出" aria-live="polite">
                  {output.chunks.map((chunk) => <li key={chunk.key} className={`output-line output-line--${chunk.stream}`}><span>{chunk.stream}</span><pre>{chunk.text}</pre></li>)}
                  {output.chunks.length === 0 ? <li className="output-empty">尚无输出。启动后由 Rust Channel 推送纯文本 Batch。</li> : null}
                </ol>
                {error ? <div className="execution-error" role="alert"><strong>{error.code}</strong><p>{error.message}</p></div> : null}
                {result ? <div className={`execution-result execution-result--${result.kind}`} role="status"><strong>{result.kind === "finished" ? "任务自然结束" : result.kind === "cancelled" ? "任务已取消" : "任务内部失败"}</strong><dl><div><dt>耗时</dt><dd>{result.durationMs} ms</dd></div>{result.exitCode !== undefined ? <div><dt>Exit Code</dt><dd>{result.exitCode}</dd></div> : null}<div><dt>终态丢弃</dt><dd>{result.droppedOutputBytes} bytes</dd></div></dl>{result.message ? <p>{result.message}</p> : null}</div> : null}
              </section>
            </div>

            <aside className="annotation-column" aria-label="执行边界与可靠性说明">
              <section className="safety-decision">
                <h2>执行所有权</h2>
                <div className={`safety-state${gateway ? "" : " safety-state--stale"}`}>{gateway ? <ShieldCheckIcon size={33} weight="light" aria-hidden="true" /> : <WarningIcon size={31} weight="light" aria-hidden="true" />}<strong>{gateway ? "Rust Core 已连接" : "仅浏览器预览"}</strong></div>
                <div className="identity-note"><ArrowBendRightDownIcon className="annotation-arrow" size={72} weight="thin" aria-hidden="true" /><div><strong>窄 IPC</strong><p>前端只能启动固定任务，不能提交脚本、PID 或可执行文件。</p><p>取消只按 Execution ID 终止对应 Job。</p></div></div>
              </section>
              <section className="runner-note"><h2>执行器事实</h2><dl><div><dt>宿主</dt><dd>Windows PowerShell</dd></div><div><dt>模式</dt><dd>NonInteractive</dd></div><div><dt>Profile</dt><dd>Disabled</dd></div><div><dt>策略</dt><dd>Process Bypass</dd></div><div><dt>目录</dt><dd>系统 Temp</dd></div></dl><p className="runner-scope">固定脚本 · 不接收用户输入</p></section>
              <section className="preflight-note"><h2><PencilSimpleLineIcon size={17} aria-hidden="true" />Output 规则</h2><ul><li>stdout / stderr 按 Batch 到达</li><li>HTML、ANSI、URL 只显示文本</li><li>当前界面最多保留 512 KiB</li></ul><p className="risk-callout">UI 变慢或断开不会反向阻塞外部任务。</p></section>
              <section className="revision-note"><h2>当前边界</h2><p>单元：CMD-01</p><p>命令：Fixed Built-in</p><p>下一步：Typed Preview</p></section>
            </aside>
          </div>

          <footer className="workspace-actions">
            <div className="execution-summary"><TerminalWindowIcon size={22} weight="light" aria-hidden="true" /><span>{gateway ? phase === "finished" ? "当前 Execution 已有后端终态，可再次运行固定验收任务。" : "任务由 Rust Core 管理；关闭 Core 会终止受管进程树。" : "当前是纯浏览器环境。请通过 dev.cmd 启动 CmdBox 桌面宿主。"}</span></div>
            <div className="action-buttons">
              <button type="button" className="secondary-button" onClick={clearOutput} disabled={output.chunks.length === 0}>清空输出</button>
              {phase === "running" || phase === "cancelling" ? <button type="button" className="cancel-button" onClick={cancelExecution} disabled={phase === "cancelling" || cancelRequestPending}><XIcon size={18} aria-hidden="true" />{phase === "cancelling" ? "正在终止" : cancelRequestPending ? "正在请求" : "终止任务"}</button> : <button type="button" className="primary-button" onClick={startExecution} disabled={!gateway || phase === "starting"}><TerminalWindowIcon size={19} aria-hidden="true" />{phase === "starting" ? "正在启动" : phase === "finished" ? "再次运行" : "运行验收任务"}</button>}
            </div>
          </footer>
        </section>
      </div>
    </main>
  );
}
