"use strict";

// zhs mariadb-crud 前端逻辑:登录态守卫、列表分页/搜索、增删改查表单。

const TOKEN_KEY = "zhs_crud_token";
const USER_KEY = "zhs_crud_user";

const state = {
    page: 1,
    pageSize: 10,
    total: 0,
    keyword: "",
    editingId: null, // null = 新增模式
};

// ---------- 登录态 ----------

if (!localStorage.getItem(TOKEN_KEY)) {
    location.replace("/index.html");
}

const currentUser = JSON.parse(localStorage.getItem(USER_KEY) || "null");
if (currentUser) {
    document.getElementById("current-user").textContent = currentUser.display_name || currentUser.username;
    document.getElementById("current-role").textContent = currentUser.role === "admin" ? "管理员" : "普通用户";
}

document.getElementById("logout-button").addEventListener("click", () => {
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
    location.replace("/index.html");
});

// ---------- API 封装 ----------

async function api(path, options = {}) {
    const response = await fetch(path, {
        ...options,
        headers: {
            "content-type": "application/json",
            "authorization": `Bearer ${localStorage.getItem(TOKEN_KEY)}`,
            ...(options.headers || {}),
        },
    });

    // 令牌缺失/过期:回到登录页
    if (response.status === 401) {
        localStorage.removeItem(TOKEN_KEY);
        localStorage.removeItem(USER_KEY);
        location.replace("/index.html");
        throw new Error("登录已过期");
    }

    return response;
}

async function apiJson(path, options = {}) {
    const response = await api(path, options);
    const data = await response.json().catch(() => null);

    if (!response.ok) {
        throw new Error((data && (data.detail || data.title)) || `请求失败 (${response.status})`);
    }

    return data;
}

// ---------- 提示 ----------

let toastTimer = null;

function toast(message, isError = false) {
    const element = document.getElementById("toast");
    element.textContent = message;
    element.className = isError ? "toast visible error" : "toast visible";
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
        element.className = "toast";
    }, 2400);
}

// ---------- 列表 ----------

function escapeHtml(text) {
    return String(text).replace(/[&<>"']/g, (ch) => {
        switch (ch) {
            case "&": return "&amp;";
            case "<": return "&lt;";
            case ">": return "&gt;";
            case '"': return "&quot;";
            default: return "&#39;";
        }
    });
}

async function loadList() {
    const query = new URLSearchParams({
        page: String(state.page),
        page_size: String(state.pageSize),
    });
    if (state.keyword) {
        query.set("keyword", state.keyword);
    }

    try {
        const data = await apiJson(`/api/products?${query}`);
        state.total = data.total;

        renderTable(data.items);
        renderPager();
    } catch (error) {
        toast(error.message, true);
        renderTable([]);
        renderPager();
    }
}

function renderTable(items) {
    const body = document.getElementById("table-body");

    if (!items.length) {
        body.innerHTML = '<tr class="empty-row"><td colspan="7">暂无数据,点击右上角“新增商品”</td></tr>';
        return;
    }

    body.innerHTML = items.map((item) => `
        <tr>
            <td>${item.id}</td>
            <td>${escapeHtml(item.name)}</td>
            <td>${escapeHtml(item.category)}</td>
            <td>${Number(item.price).toFixed(2)}</td>
            <td>${item.quantity}</td>
            <td>${escapeHtml(item.updated_at)}</td>
            <td class="actions">
                <button class="btn-link" type="button" data-action="edit" data-id="${item.id}">编辑</button>
                <button class="btn-danger" type="button" data-action="delete" data-id="${item.id}" data-name="${escapeHtml(item.name)}">删除</button>
            </td>
        </tr>`).join("");
}

function renderPager() {
    const pages = Math.max(1, Math.ceil(state.total / state.pageSize));
    document.getElementById("page-info").textContent = `共 ${state.total} 条,第 ${state.page}/${pages} 页`;
    document.getElementById("page-number").textContent = String(state.page);
    document.getElementById("prev-button").disabled = state.page <= 1;
    document.getElementById("next-button").disabled = state.page >= pages;
}

// ---------- 表单 ----------

const formPanel = document.getElementById("form-panel");
const formTitle = document.getElementById("form-title");
const productForm = document.getElementById("product-form");

function openForm(product) {
    state.editingId = product ? product.id : null;
    formTitle.textContent = product ? `编辑商品 #${product.id}` : "新增商品";
    document.getElementById("field-name").value = product ? product.name : "";
    document.getElementById("field-category").value = product ? product.category : "";
    document.getElementById("field-price").value = product ? product.price : "";
    document.getElementById("field-quantity").value = product ? product.quantity : "";
    formPanel.classList.remove("hidden");
    document.getElementById("field-name").focus();
}

function closeForm() {
    state.editingId = null;
    formPanel.classList.add("hidden");
    productForm.reset();
}

productForm.addEventListener("submit", async (event) => {
    event.preventDefault();

    const payload = {
        name: document.getElementById("field-name").value.trim(),
        category: document.getElementById("field-category").value.trim(),
        price: Number(document.getElementById("field-price").value),
        quantity: Number.parseInt(document.getElementById("field-quantity").value, 10),
    };

    if (!payload.name || !payload.category) {
        toast("名称和分类不能为空", true);
        return;
    }
    if (!Number.isFinite(payload.price) || payload.price < 0) {
        toast("单价必须是不小于 0 的数字", true);
        return;
    }
    if (!Number.isInteger(payload.quantity) || payload.quantity < 0) {
        toast("数量必须是不小于 0 的整数", true);
        return;
    }

    const button = document.getElementById("save-button");
    button.disabled = true;

    try {
        if (state.editingId === null) {
            await apiJson("/api/products", { method: "POST", body: JSON.stringify(payload) });
            toast("新增成功");
            state.page = 1; // 列表按 id 倒序,新记录在第 1 页
        } else {
            await apiJson(`/api/products/${state.editingId}`, { method: "PUT", body: JSON.stringify(payload) });
            toast("保存成功");
        }

        closeForm();
        await loadList();
    } catch (error) {
        toast(error.message, true);
    } finally {
        button.disabled = false;
    }
});

// ---------- 事件绑定 ----------

document.getElementById("table-body").addEventListener("click", async (event) => {
    const button = event.target.closest("button[data-action]");
    if (!button) {
        return;
    }

    const id = Number(button.dataset.id);

    if (button.dataset.action === "edit") {
        try {
            const product = await apiJson(`/api/products/${id}`);
            openForm(product);
        } catch (error) {
            toast(error.message, true);
        }
        return;
    }

    if (button.dataset.action === "delete") {
        if (!confirm(`确认删除“${button.dataset.name}”吗?`)) {
            return;
        }

        try {
            const response = await api(`/api/products/${id}`, { method: "DELETE" });
            if (!response.ok && response.status !== 404) {
                const data = await response.json().catch(() => null);
                throw new Error((data && (data.detail || data.title)) || `删除失败 (${response.status})`);
            }
            toast("删除成功");
            await loadList();
        } catch (error) {
            toast(error.message, true);
        }
    }
});

document.getElementById("create-button").addEventListener("click", () => openForm(null));
document.getElementById("cancel-button").addEventListener("click", closeForm);

document.getElementById("search-button").addEventListener("click", () => {
    state.keyword = document.getElementById("keyword").value.trim();
    state.page = 1;
    loadList();
});

document.getElementById("keyword").addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
        document.getElementById("search-button").click();
    }
});

document.getElementById("reset-button").addEventListener("click", () => {
    document.getElementById("keyword").value = "";
    state.keyword = "";
    state.page = 1;
    loadList();
});

document.getElementById("prev-button").addEventListener("click", () => {
    state.page -= 1;
    loadList();
});

document.getElementById("next-button").addEventListener("click", () => {
    state.page += 1;
    loadList();
});

// ---------- 初始化 ----------

loadList();
