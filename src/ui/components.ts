export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  for (const child of children) {
    node.append(child instanceof Node ? child : document.createTextNode(child));
  }
  return node;
}

export function iconStatus(ok: boolean, warn = false): HTMLElement {
  const dot = el("span", "status-dot");
  if (ok) {
    dot.style.cssText = "background:#4caf50; box-shadow:0 0 10px rgba(76,175,80,0.5)";
    dot.classList.add("pulsing");
  } else if (warn) {
    dot.style.cssText = "background:#e8b87a; box-shadow:0 0 8px rgba(232,184,122,0.5)";
  } else {
    dot.style.cssText = "background:#df3e2b; box-shadow:0 0 8px rgba(223,62,43,0.35)";
  }
  return dot;
}

export async function copyText(text: string, button: HTMLButtonElement): Promise<void> {
  await navigator.clipboard.writeText(text);
  const original = button.textContent;
  button.textContent = "✓ Copied";
  button.style.background = "#e8f5e9";
  button.style.borderColor = "#a5d6a7";
  button.style.color = "#2e7d32";
  setTimeout(() => {
    button.textContent = original;
    button.style.background = "";
    button.style.borderColor = "";
    button.style.color = "";
  }, 1800);
}

export function toast(message: string, type: "success" | "error" | "info" = "info"): void {
  const root = document.getElementById("toasts");
  if (!root) return;

  const accents: Record<string, string> = {
    success: "#4caf50",
    error:   "#df3e2b",
    info:    "#e8b87a",
  };
  const icons: Record<string, string> = {
    success: "✓",
    error:   "✕",
    info:    "•",
  };

  const node = el("div", "animate-slide_up flex items-start gap-3 px-4 py-3 rounded-feex-sm text-sm");
  node.style.cssText = `
    background: #ffffff;
    border: 1px solid #ece6e6;
    border-left: 3px solid ${accents[type]};
    box-shadow: 0 8px 30px rgba(0,0,0,0.12);
    color: #5a4a48;
  `;

  const icon = el("span", "shrink-0 font-bold text-base leading-5");
  icon.style.color = accents[type];
  icon.textContent = icons[type];

  const msg = el("span", "flex-1 leading-5");
  msg.textContent = message;

  node.append(icon, msg);
  root.append(node);
  setTimeout(() => {
    node.style.transition = "opacity 0.3s, transform 0.3s";
    node.style.opacity = "0";
    node.style.transform = "translateX(12px)";
    setTimeout(() => node.remove(), 320);
  }, 4000);
}

export function settingCard(
  label: string,
  value: string,
  onCopy?: () => void,
): HTMLElement {
  const card = el("div", "rounded-feex-sm p-3.5");
  card.style.cssText = "background:#faf7f7; border:1px solid #ece6e6; transition:border-color 0.2s, background 0.2s";
  card.addEventListener("mouseenter", () => { card.style.borderColor = "#f3ddc7"; card.style.background = "#fffdfb"; });
  card.addEventListener("mouseleave", () => { card.style.borderColor = "#ece6e6"; card.style.background = "#faf7f7"; });

  const lbl = el("div", "text-xs uppercase tracking-widest mb-2 font-medium");
  lbl.style.color = "#8a8a8a";
  lbl.textContent = label;

  const row = el("div", "flex items-center gap-3");
  const val = el("code", "flex-1 text-sm font-mono break-all");
  val.style.color = "#5a4a48";
  val.textContent = value;
  row.append(val);

  if (onCopy) {
    const btn = el("button", "copy-btn shrink-0") as HTMLButtonElement;
    btn.textContent = "Copy";
    btn.addEventListener("click", () => onCopy());
    row.append(btn);
  }

  card.append(lbl, row);
  return card;
}
