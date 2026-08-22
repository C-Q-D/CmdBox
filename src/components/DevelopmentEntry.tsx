/** CmdBox 环境准备页中可复用的开发入口卡片。 */

/** 开发入口卡片所需的稳定展示信息。 */
interface DevelopmentEntryProps {
  /** 入口所属的开发范围。 */
  label: string;
  /** 入口的主要名称。 */
  title: string;
  /** 入口适用场景的简短说明。 */
  description: string;
}

/** 渲染一个开发入口，不持有状态或业务行为。 */
export function DevelopmentEntry({
  label,
  title,
  description,
}: DevelopmentEntryProps) {
  return (
    <article>
      <span>{label}</span>
      <strong>{title}</strong>
      <p>{description}</p>
    </article>
  );
}
