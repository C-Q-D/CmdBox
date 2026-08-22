import { ArchiveBoxIcon } from "@phosphor-icons/react/dist/csr/BoxArrowDown";
import { ArrowBendRightDownIcon } from "@phosphor-icons/react/dist/csr/ArrowBendRightDown";
import { CalendarBlankIcon } from "@phosphor-icons/react/dist/csr/CalendarBlank";
import { ClockCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ClockCounterClockwise";
import { CommandIcon } from "@phosphor-icons/react/dist/csr/Command";
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
import { TrashIcon } from "@phosphor-icons/react/dist/csr/Trash";
import { WarningIcon } from "@phosphor-icons/react/dist/csr/Warning";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import cmdboxIcon from "../../../src-tauri/icons/icon.png";
import {
  prototypeCommands,
  prototypePreview,
  prototypeTargets,
  type PrototypeCommandSummary,
} from "./prototype-data";

/** 为命令摘要选择统一图标库中的稳定图标。 */
function CommandSummaryIcon({ icon }: Pick<PrototypeCommandSummary, "icon">) {
  const props = { "aria-hidden": true, size: 19, weight: "light" } as const;

  switch (icon) {
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

/** 渲染项目已确认的三栏 Command Workspace 前端视觉原型。 */
export function CommandWorkspacePrototype() {
  return (
    <main className="prototype-shell">
      <header className="window-bar">
        <div className="brand-lockup">
          <img src={cmdboxIcon} alt="" className="brand-mark" />
          <span className="brand-name">CmdBox</span>
          <span className="version-mark">v1.0.0</span>
        </div>
        <div className="window-caption" aria-hidden="true">
          <MinusIcon size={16} weight="light" />
          <SquareIcon size={13} weight="light" />
          <XIcon size={16} weight="light" />
        </div>
      </header>

      <div className="workspace-grid">
        <nav className="global-navigation" aria-label="主导航">
          <p className="rail-label">导航</p>
          <div className="global-navigation__items">
            <a className="rail-link rail-link--active" href="#command-workspace">
              <CommandIcon size={21} weight="light" aria-hidden="true" />
              <span>命令</span>
            </a>
            <a className="rail-link" href="#templates">
              <FileTextIcon size={21} weight="light" aria-hidden="true" />
              <span>模板</span>
            </a>
            <a className="rail-link" href="#environments">
              <GitBranchIcon size={21} weight="light" aria-hidden="true" />
              <span>环境</span>
            </a>
            <a className="rail-link" href="#schedules">
              <CalendarBlankIcon size={21} weight="light" aria-hidden="true" />
              <span>计划</span>
            </a>
            <a className="rail-link" href="#history">
              <ClockCounterClockwiseIcon size={21} weight="light" aria-hidden="true" />
              <span>历史</span>
            </a>
            <a className="rail-link" href="#settings">
              <GearIcon size={21} weight="light" aria-hidden="true" />
              <span>设置</span>
            </a>
          </div>

          <div className="rail-footer">
            <img src={cmdboxIcon} alt="CmdBox" className="rail-footer__mark" />
            <p className="mono-label">CMD BOX</p>
            <p>在命令行确定性之间，建立可重复的一次性桥梁。</p>
            <div className="rail-footer__platform">
              <span>Windows 优先</span>
              <span>本地执行 · 本地安全</span>
            </div>
          </div>
        </nav>

        <aside className="command-index" aria-label="Command Block 索引">
          <div className="command-index__heading">
            <span>命令块索引</span>
            <button type="button" className="text-action">
              <PlusIcon size={16} aria-hidden="true" />
              新建
            </button>
          </div>

          <label className="search-field">
            <span className="visually-hidden">搜索命令块</span>
            <MagnifyingGlassIcon size={18} aria-hidden="true" />
            <input type="search" placeholder="搜索命令块…" />
          </label>

          <p className="index-caption">全部命令块</p>
          <ul className="command-list">
            {prototypeCommands.map((command) => (
              <li key={command.id}>
                <button
                  type="button"
                  className={`command-row${command.selected ? " command-row--selected" : ""}`}
                  aria-current={command.selected ? "page" : undefined}
                >
                  <span className="command-row__icon">
                    <CommandSummaryIcon icon={command.icon} />
                  </span>
                  <span className="command-row__body">
                    <strong>{command.name}</strong>
                    <small>
                      就绪 <span aria-hidden="true">·</span> {command.runner}
                    </small>
                  </span>
                </button>
              </li>
            ))}
          </ul>
          <p className="command-count">共 {prototypeCommands.length} 个命令块</p>
        </aside>

        <section className="command-workspace" id="command-workspace">
          <header className="workspace-heading">
            <p className="workspace-breadcrumb">
              命令工作区 <span>/</span> 快速永久删除多个文件夹
            </p>
            <h1>快速永久删除多个文件夹</h1>
            <p>永久删除指定的多个文件夹。注意：删除后不可恢复，不经过回收站。</p>
          </header>

          <div className="workspace-content">
            <div className="evidence-column">
              <section className="runner-facts" aria-label="预览状态">
                <div>
                  <span>执行器</span>
                  <strong>Windows PowerShell</strong>
                </div>
                <div>
                  <span>状态</span>
                  <strong className="state-ready">预览已就绪</strong>
                </div>
              </section>

              <section className="target-record" aria-labelledby="target-title">
                <div className="section-heading-row">
                  <h2 id="target-title">目标文件夹（3）</h2>
                  <button type="button" className="text-action text-action--add">
                    <PlusIcon size={16} aria-hidden="true" />
                    添加文件夹
                  </button>
                </div>
                <ol className="target-list">
                  {prototypeTargets.map((path, index) => (
                    <li key={path}>
                      <span className="target-index">{index + 1}</span>
                      <code>{path}</code>
                      <button type="button" aria-label={`移除 ${path}`}>
                        <XIcon size={15} aria-hidden="true" />
                      </button>
                    </li>
                  ))}
                </ol>
              </section>

              <section className="preview-record" aria-labelledby="preview-title">
                <div className="preview-summary">
                  <span>预览摘要</span>
                  <strong>3 个目标 · 已规范化</strong>
                </div>
                <h2 id="preview-title">预览脚本（PowerShell）</h2>
                <pre aria-label="PowerShell 命令预览"><code>{prototypePreview}</code></pre>
                <p className="preview-footnote">
                  说明：使用 <code>-LiteralPath</code> 确保路径按字面值处理，避免通配符影响。
                </p>
              </section>
            </div>

            <aside className="annotation-column" aria-label="安全决策与执行说明">
              <section className="safety-decision">
                <h2>安全决策</h2>
                <div className="safety-state">
                  <ShieldCheckIcon size={33} weight="light" aria-hidden="true" />
                  <strong>安全检查通过</strong>
                </div>
                <div className="identity-note">
                  <ArrowBendRightDownIcon
                    className="annotation-arrow"
                    size={36}
                    weight="thin"
                    aria-hidden="true"
                  />
                  <div>
                    <strong>身份核对</strong>
                    <p>将永久删除左侧 3 个文件夹及其所有内容。</p>
                    <p>不经过回收站。</p>
                  </div>
                </div>
              </section>

              <section className="runner-note">
                <h2>执行器事实</h2>
                <dl>
                  <div><dt>宿主</dt><dd>Windows PowerShell</dd></div>
                  <div><dt>位数</dt><dd>64-bit</dd></div>
                  <div><dt>策略</dt><dd>Bypass</dd></div>
                  <div><dt>作用域</dt><dd>本地执行</dd></div>
                </dl>
              </section>

              <section className="preflight-note">
                <h2><PencilSimpleLineIcon size={17} aria-hidden="true" />执行前记录</h2>
                <p>请在执行前核对：</p>
                <ul>
                  <li>目标路径是否准确</li>
                  <li>是否已备份关键数据</li>
                  <li>当前会话具备删除权限</li>
                </ul>
                <p className="risk-callout">以上核对直接关联左侧目标与预览脚本。</p>
              </section>

              <section className="revision-note">
                <h2>修订记录</h2>
                <p>定义：Built-in Command</p>
                <p>Preview：当前 revision</p>
                <p>参数变化后需重新预览</p>
              </section>
            </aside>
          </div>

          <footer className="workspace-actions">
            <div className="destructive-summary">
              <WarningIcon size={22} weight="light" aria-hidden="true" />
              <span>永久删除，不经过回收站。请确认以上 3 个目标及脚本无误后执行。</span>
            </div>
            <div className="action-buttons">
              <button type="button" className="secondary-button">清空所选</button>
              <button type="button" className="destructive-button">
                <TrashIcon size={20} aria-hidden="true" />永久删除
              </button>
            </div>
          </footer>
        </section>
      </div>
    </main>
  );
}
