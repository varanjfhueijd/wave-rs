// Branchement de la maquette sur l'API locale du SDK wave-rs.
// Ce script est injecté APRÈS le script de la maquette : les fonctions
// redéclarées ici (showView, simulatePay) remplacent les versions factices.

async function api(path, opts) {
  const response = await fetch(path, opts);
  const payload = await response.json().catch(() => ({ error: "réponse invalide du serveur" }));
  if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
  return payload;
}

const STATUS_FR = {
  pending: { label: "en attente", badge: "badge-warn" },
  successful: { label: "succès", badge: "badge-success" },
  failed: { label: "échouée", badge: "badge-muted" },
  cancelled: { label: "annulée", badge: "badge-muted" },
  expired: { label: "expirée", badge: "badge-muted" },
};

const PROVIDER_COLORS = { wave: "#1D9E75", orange: "#EF9F27", mtn: "#378ADD", moov: "#7F77DD" };

let providerCatalog = [];

// --- Disponibilité réelle des providers (sidebar + formulaire) -------------

async function refreshProviders() {
  try {
    providerCatalog = await api("/api/providers");
  } catch (err) {
    console.warn("providers:", err);
    return;
  }
  // Les 4 items de la sidebar sont dans le même ordre que le catalogue.
  const badges = document.querySelectorAll(".nav-item .badge");
  providerCatalog.forEach((p, i) => {
    const badge = badges[i];
    if (!badge) return;
    badge.textContent = p.available ? "actif" : "non configuré";
    badge.className = "badge " + (p.available ? "badge-success" : "badge-muted");
    badge.style.marginLeft = "auto";
    badge.style.fontSize = "10px";
  });
  const rebuild = (select) => {
    if (!select) return;
    select.innerHTML = providerCatalog
      .map(
        (p) =>
          `<option value="${p.name}" ${p.available ? "" : "disabled"}>` +
          `${p.name}${p.available ? "" : " (non configuré)"}</option>`
      )
      .join("");
    const first = providerCatalog.find((p) => p.available);
    if (first) select.value = first.name;
  };
  rebuild(document.getElementById("pay-provider"));
  rebuild(document.getElementById("tx-provider"));
}

// --- Paiement réel via POST /api/pay ---------------------------------------

async function simulatePay() {
  const provider = document.getElementById("pay-provider").value;
  const phone = document.getElementById("pay-phone").value || "+2250700000000";
  const amount = parseInt(document.getElementById("pay-amount").value || "5000", 10);
  const note = document.getElementById("pay-note").value || null;
  const result = document.getElementById("pay-result");
  result.style.display = "";
  result.innerHTML = '<span style="color:#EF9F27">⟳ Paiement en cours… (timeout 90s)</span>';
  try {
    const reply = await api("/api/pay", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ provider, to: phone, amount, note }),
    });
    const status = STATUS_FR[reply.status] || { label: reply.status };
    result.innerHTML =
      `<span style="color:#1D9E75">✔ ${status.label}</span>\n` +
      `<span style="color:var(--text-muted)">id:       </span>${reply.transaction_id}\n` +
      `<span style="color:var(--text-muted)">provider: </span>${reply.provider}\n` +
      `<span style="color:var(--text-muted)">to:       </span>${phone}\n` +
      `<span style="color:var(--text-muted)">amount:   </span><span style="color:#1D9E75">${amount.toLocaleString()} XOF</span>`;
  } catch (err) {
    result.innerHTML = `<span style="color:#E24B4A">✘ ${err.message}</span>`;
  }
}

// --- Transactions réelles ---------------------------------------------------

function renderLiveTx(transactions) {
  const tbody = document.getElementById("tx-body");
  if (!transactions.length) {
    tbody.innerHTML =
      '<tr><td colspan="6" style="color:var(--text-muted);font-size:12px">Aucune transaction.</td></tr>';
    return;
  }
  tbody.innerHTML = transactions
    .map((t) => {
      const status = STATUS_FR[t.status] || { label: t.status, badge: "badge-muted" };
      const color = PROVIDER_COLORS[t.provider] || "#B4B2A9";
      return (
        `<tr><td class="tx-id">${t.id}</td>` +
        `<td class="amount-positive">${t.amount.amount.toLocaleString()} ${"XOF"}</td>` +
        `<td style="font-size:12px;color:var(--text-secondary)">${t.counterparty || "—"}</td>` +
        `<td><span class="provider-pill"><span class="status-dot" style="background:${color}"></span>${t.provider}</span></td>` +
        `<td style="font-size:11px;color:var(--text-muted)">${t.created_at || "—"}</td>` +
        `<td><span class="badge ${status.badge}">${status.label}</span></td></tr>`
      );
    })
    .join("");
}

async function loadTx() {
  const provider = document.getElementById("tx-provider").value;
  const account = document.getElementById("tx-account").value || "+2250700000000";
  const tbody = document.getElementById("tx-body");
  tbody.innerHTML =
    '<tr><td colspan="6" style="color:var(--text-muted);font-size:12px">Chargement…</td></tr>';
  try {
    const transactions = await api(
      `/api/transactions?provider=${encodeURIComponent(provider)}&account=${encodeURIComponent(account)}&limit=25`
    );
    renderLiveTx(transactions);
  } catch (err) {
    tbody.innerHTML = `<tr><td colspan="6" style="color:#E24B4A;font-size:12px">✘ ${err.message}</td></tr>`;
  }
}

// Barre provider/compte injectée au-dessus de l'historique.
function installTxToolbar() {
  const view = document.getElementById("view-transactions");
  const card = view.querySelector(".card");
  const bar = document.createElement("div");
  bar.style.cssText = "display:flex;gap:8px;margin-bottom:8px;align-items:center";
  bar.innerHTML =
    '<select id="tx-provider" style="padding:6px 10px;font-size:12px;border:0.5px solid var(--border-strong);border-radius:var(--radius);background:var(--surface-1);color:var(--text-primary)"></select>' +
    '<input id="tx-account" type="text" value="+2250700000000" style="flex:1;padding:6px 10px;font-size:12px;border:0.5px solid var(--border-strong);border-radius:var(--radius);background:var(--surface-1);color:var(--text-primary)" />' +
    '<button class="btn-sm" onclick="loadTx()"><i class="ti ti-refresh" aria-hidden="true"></i> Charger</button>';
  view.insertBefore(bar, card);
}

// showView sans données factices : l'historique se charge via l'API.
function showView(name) {
  ["dashboard", "payment", "providers", "transactions", "cli", "docs"].forEach((v) => {
    document.getElementById("view-" + v).style.display = "none";
  });
  document.getElementById("view-" + name).style.display = "";
  const titles = {
    dashboard: "Dashboard",
    payment: "Envoyer un paiement",
    providers: "Providers",
    transactions: "Transactions",
    cli: "CLI",
    docs: "Documentation",
  };
  document.getElementById("view-title").textContent = titles[name] || name;
  document.querySelectorAll(".nav-item").forEach((el) => el.classList.remove("active"));
  if (name === "transactions") loadTx();
}

installTxToolbar();
refreshProviders();
