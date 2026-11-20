// Tooltips of both pages, drawn by the page instead of the browser: after a short hover with the
// mouse, at once on a focus from the keyboard, and when a field to type in takes the focus (the
// only way to see them on a phone).
//
// The text is the `title` of an element, or of the <label> around a control. This script moves
// it to `data-tip`, so that the browser does not draw its own tooltip as well, and keeps it as
// the description of the control (`aria-describedby`) for the screen readers.
//
// WCAG 1.4.13: Escape hides the tooltip without moving the pointer or the focus, the pointer can
// move onto the tooltip, and the tooltip stays until the pointer or the focus leaves.
"use strict";

(() => {
    const HOVER_DELAY_MS = 400;
    const LEAVE_DELAY_MS = 300;
    // Space kept between the tooltip, its element and the edges of the screen
    const MARGIN = 8;
    // The fields a phone focuses with a tap, to type in
    const TYPING = new Set(["text", "password", "number", "search", "email", "url", "tel"]);

    const tip = document.createElement("div");
    tip.className = "tooltip";
    // Screen readers read the description of the control, not this copy
    tip.setAttribute("aria-hidden", "true");
    tip.hidden = true;

    let target = null;          // The element the tooltip is shown for
    let shownBy = null;         // "hover" or "focus"
    let dismissed = null;       // Hidden with Escape: not shown again while the pointer stays on it
    let timer = null;
    let lastInput = "pointer";  // "keyboard" once a key is pressed, "pointer" once the pointer presses
    let descriptions = 0;

    // The control that a <label title> describes, or the element itself
    function describedBy(element) {
        return element.tagName === "LABEL" && element.control ? element.control : element;
    }

    // Moves the title of an element to data-tip, and keeps it as the description of its control
    function adopt(element) {
        const text = element.getAttribute("title");
        if (text === null) {
            return;
        }
        element.removeAttribute("title");
        element.dataset.tip = text.trim();
        let node = element.dataset.tipId ? document.getElementById(element.dataset.tipId) : null;
        if (!node) {
            node = document.createElement("span");
            node.id = `tip-${++descriptions}`;
            // Hidden from reading in page order: a description is read even when hidden
            node.hidden = true;
            (element.closest("dialog") || document.body).appendChild(node);
            element.dataset.tipId = node.id;
            const control = describedBy(element);
            const ids = (control.getAttribute("aria-describedby") || "").split(/\s+/).filter(Boolean);
            control.setAttribute("aria-describedby", [...ids, node.id].join(" "));
        }
        node.textContent = element.dataset.tip;
        if (element === target) {
            tip.textContent = element.dataset.tip;
        }
    }

    // Every element of `root` that still has a title, `root` included
    function adoptAll(root) {
        if (!(root instanceof Element)) {
            return;
        }
        if (root.hasAttribute("title")) {
            adopt(root);
        }
        root.querySelectorAll("[title]").forEach(adopt);
    }

    // The element whose tooltip a node of the page shows, if any
    function tipped(node) {
        const element = node instanceof Element ? node.closest("[title], [data-tip]") : null;
        if (element) {
            adopt(element);
        }
        return element && element.dataset.tip ? element : null;
    }

    // Under the element, or above it when there is no room below, always inside the screen
    function place() {
        tip.style.maxWidth = `${Math.min(320, window.innerWidth - 2 * MARGIN)}px`;
        tip.style.left = "0px";
        tip.style.top = "0px";
        const box = target.getBoundingClientRect();
        const width = tip.offsetWidth;
        const height = tip.offsetHeight;
        let top = box.bottom + MARGIN;
        if (top + height > window.innerHeight - MARGIN && box.top - MARGIN - height >= MARGIN) {
            top = box.top - MARGIN - height;
        }
        const left = Math.max(MARGIN, Math.min(box.left + box.width / 2 - width / 2, window.innerWidth - MARGIN - width));
        tip.style.left = `${Math.round(left)}px`;
        tip.style.top = `${Math.round(top)}px`;
    }

    function show(element, by) {
        clearTimeout(timer);
        target = element;
        shownBy = by;
        tip.textContent = element.dataset.tip;
        // A modal dialog lies over the rest of the page: the tooltip of one of its controls goes inside it
        (element.closest("dialog[open]") || document.body).appendChild(tip);
        tip.hidden = false;
        place();
    }

    function hide() {
        clearTimeout(timer);
        timer = null;
        target = null;
        shownBy = null;
        tip.hidden = true;
    }

    document.addEventListener("keydown", event => {
        if (event.key === "Escape" && !tip.hidden) {
            // The first Escape only hides the tooltip: the next one closes a dialog
            dismissed = target;
            hide();
            event.preventDefault();
            event.stopPropagation();
            return;
        }
        lastInput = "keyboard";
    }, true);

    document.addEventListener("pointerdown", () => {
        lastInput = "pointer";
        if (shownBy === "hover") {
            dismissed = target;
            hide();
        }
    }, true);

    document.addEventListener("pointerover", event => {
        // A tap is not a hover: on a phone, the tooltips come with the focus of a field
        if (event.pointerType === "touch" || tip.contains(event.target)) {
            clearTimeout(timer);
            return;
        }
        const element = tipped(event.target);
        if (element !== dismissed) {
            dismissed = null;
        }
        if (element && (element === target || element === dismissed)) {
            clearTimeout(timer);
            return;
        }
        clearTimeout(timer);
        if (!element) {
            // Time to cross the gap between the element and its tooltip
            if (shownBy === "hover") {
                timer = setTimeout(hide, LEAVE_DELAY_MS);
            }
            return;
        }
        // From one tooltip to the next without waiting again
        if (shownBy === "hover") {
            show(element, "hover");
        } else {
            timer = setTimeout(() => show(element, "hover"), HOVER_DELAY_MS);
        }
    });

    document.addEventListener("pointerout", event => {
        // Out of the window
        if (!event.relatedTarget) {
            clearTimeout(timer);
            if (shownBy === "hover") {
                hide();
            }
        }
    });

    document.addEventListener("focusin", event => {
        const element = tipped(event.target);
        if (!element) {
            return;
        }
        const typing = event.target.tagName === "INPUT" && TYPING.has(event.target.type);
        if (lastInput === "keyboard" || typing) {
            dismissed = null;
            show(element, "focus");
        }
    });

    document.addEventListener("focusout", event => {
        if (shownBy === "focus" && target && target.contains(event.target)) {
            hide();
        }
    });

    // The tooltip follows its element: a phone scrolls the page when its keyboard opens
    const follow = () => {
        if (target) {
            place();
        }
    };
    window.addEventListener("scroll", follow, true);
    window.addEventListener("resize", follow);

    // The titles of the pages, and those the scripts add or change later
    adoptAll(document.body);
    new MutationObserver(mutations => {
        for (const mutation of mutations) {
            if (mutation.type === "attributes") {
                adopt(mutation.target);
            } else {
                mutation.addedNodes.forEach(adoptAll);
            }
        }
    }).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["title"] });
})();
