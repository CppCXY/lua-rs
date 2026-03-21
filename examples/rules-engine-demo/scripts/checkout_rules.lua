local function add_tag(decision, tag)
    table.insert(decision.tags, tag)
end

local function has_category(order, category)
    for _, item in ipairs(order.items) do
        if item.category == category then
            return true
        end
    end
    return false
end

function evaluate_order(order)
    audit(string.format(
        "evaluating order=%s total=%.2f customer=%s",
        order.id,
        order.total_cents / 100,
        order.customer.email
    ))

    local decision = {
        approved = true,
        action = "approve",
        discount_cents = 0,
        shipping_tier = "standard",
        eta_days = 0,
        reason = "default approval",
        tags = {},
    }

    for _, item in ipairs(order.items) do
        if not inventory_available(item.sku, item.quantity) then
            decision.approved = false
            decision.action = "reject"
            decision.reason = "inventory unavailable for " .. item.sku
            add_tag(decision, "inventory_block")
            return decision
        end
    end

    local risk = risk_score(order.customer.email, order.total_cents, order.customer.country)
    add_tag(decision, "risk:" .. tostring(risk))

    if order.customer.country == "BR" and has_category(order, "hazmat") then
        decision.approved = false
        decision.action = "manual_review"
        decision.reason = "hazmat export requires compliance review"
        decision.shipping_tier = "hold"
        add_tag(decision, "compliance")
    elseif risk >= 85 then
        decision.approved = false
        decision.action = "manual_review"
        decision.reason = "high risk checkout requires analyst approval"
        decision.shipping_tier = "hold"
        add_tag(decision, "fraud_review")
    elseif order.customer.vip and order.total_cents >= 10000 then
        decision.discount_cents = math.floor(order.total_cents * 0.15)
        decision.shipping_tier = "express"
        decision.reason = "vip customer received dynamic discount"
        add_tag(decision, "vip")
    elseif order.coupon_code == "FLASH10" and order.total_cents >= 5000 then
        decision.discount_cents = math.floor(order.total_cents * 0.10)
        decision.reason = "campaign coupon accepted"
        add_tag(decision, "campaign")
    end

    if decision.action == "approve" and order.customer.loyalty_points >= 1000 then
        decision.shipping_tier = "priority"
        add_tag(decision, "loyalty_upgrade")
    end

    if decision.action == "approve" and has_category(order, "fragile") and decision.shipping_tier == "standard" then
        decision.shipping_tier = "priority"
        add_tag(decision, "fragile_handling")
    end

    decision.eta_days = shipping_eta(order.customer.country, decision.shipping_tier)
    return decision
end