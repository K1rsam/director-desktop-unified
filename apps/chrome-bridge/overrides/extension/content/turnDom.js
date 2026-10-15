// Turn ownership and identity shared by live observation and recovery.
// Presentation-only message shapes are readable, but never invent durable IDs.
(() => {
  'use strict';

  function createTurnDom() {
    const TURN_SELECTOR = '[data-turn="user"], [data-turn="assistant"]';
    const MESSAGE_SELECTOR = '[data-message-author-role="user"], [data-message-author-role="assistant"]';

    function excluded(node) {
      return !globalThis.ChatGptDomUtilities.isPrimaryChatSurfaceElement(node)
        || Boolean(node?.closest?.('nav, aside, [role="navigation"], [role="complementary"], form, [hidden], [aria-hidden="true"], [inert], [data-testid*="composer" i], #cgb-panel'));
    }

    function markdownNodes(root) {
      if (!root) return [];
      const result = [];
      if (root.matches?.('[class*="MarkdownRoot"]') && !root.matches?.('.rich-text-user-turn')) result.push(root);
      result.push(...Array.from(root.querySelectorAll?.('[class*="MarkdownRoot"]') || []));
      return result.filter((node, index, all) => (
        !node.matches?.('.rich-text-user-turn')
        && !node.closest?.('.bg-user-message')
        && all.indexOf(node) === index
      ));
    }

    function finalAnswerNode(root) {
      const result = markdownNodes(root);
      return result[result.length - 1] || null;
    }

    function compareDocumentOrder(left, right) {
      if (left === right) return 0;
      const relation = left?.compareDocumentPosition?.(right) || 0;
      const preceding = globalThis.Node?.DOCUMENT_POSITION_PRECEDING || 2;
      const following = globalThis.Node?.DOCUMENT_POSITION_FOLLOWING || 4;
      if (relation & following) return -1;
      if (relation & preceding) return 1;
      return 0;
    }

    function semanticRole(node) {
      return node?.getAttribute?.('data-turn') || node?.getAttribute?.('data-message-author-role') || '';
    }

    function role(node) {
      const explicit = semanticRole(node);
      if (explicit === 'user' || explicit === 'assistant') return explicit;
      if (node?.matches?.('.bg-user-message')) return 'user';
      if (Array.from(node?.children || []).some((child) => child.matches?.('h4.sr-only'))
        && (markdownNodes(node).length || node.querySelector?.('[data-testid="generated-image-gallery"], [data-testid="generated-image-preview"]'))) return 'assistant';
      return '';
    }

    function getTurnNodes() {
      // Resolve each message to its owning turn. Do not switch the entire page
      // to one selector family when wrapped and unwrapped messages coexist.
      const result = [];
      const presentation = [];
      const DISCOVERY_SELECTOR = `${TURN_SELECTOR}, ${MESSAGE_SELECTOR}, .bg-user-message, h4.sr-only`;
      for (const node of Array.from(document.querySelectorAll(DISCOVERY_SELECTOR))) {
        if (excluded(node)) continue;
        if (node.matches?.(`${TURN_SELECTOR}, ${MESSAGE_SELECTOR}`)) {
          const turn = node.closest?.(TURN_SELECTOR) || node;
          if (!result.includes(turn)) result.push(turn);
        } else {
          // The agent activity panel uses the same screen-reader headings as
          // messages. Its presentation-only progress is not another answer.
          // Explicit native message/turn identities above remain authoritative.
          if (node.matches?.('h4.sr-only') && node.closest?.('[class~="group/agent-activity"]')) continue;
          const candidate = node.matches?.('h4.sr-only') ? node.parentElement : node;
          if (candidate && role(candidate)) presentation.push(candidate);
        }
      }
      for (const node of presentation) {
        if (result.some((turn) => turn === node || turn.contains?.(node) || node.contains?.(turn))) continue;
        result.push(node);
      }
      return result.sort(compareDocumentOrder);
    }

    function owner(node) {
      if (!node || excluded(node)) return null;
      const explicit = node.closest?.(TURN_SELECTOR) || node.closest?.(MESSAGE_SELECTOR);
      if (explicit) return explicit;
      for (let current = node; current; current = current.parentElement) {
        if (role(current)) return current;
      }
      return null;
    }

    function generatedImageScopes(node) {
      const turn=owner(node)||node;
      if (!turn || role(turn)!=='assistant') return node?[node]:[];
      const scopes=[turn],turns=getTurnNodes();
      const users=turns.filter(n=>role(n)==='user'),assistants=turns.filter(n=>role(n)==='assistant');
      // The presentation renderer can put its image tool result between the
      // user bubble and the final answer, outside the final answer's wrapper.
      // Expand only a unique fresh conversation, never a multi-turn history.
      if (users.length!==1 || assistants.length!==1 || assistants[0]!==turn) return scopes;
      const following=globalThis.Node?.DOCUMENT_POSITION_FOLLOWING||4;
      const galleries=Array.from(document.querySelectorAll('[data-testid="generated-image-gallery"]')).filter(gallery=>{
        if (excluded(gallery)||turn.contains?.(gallery)||users[0].contains?.(gallery)) return false;
        const afterUser=users[0].compareDocumentPosition?.(gallery)||0;
        const beforeAnswer=gallery.compareDocumentPosition?.(turn)||0;
        return !(afterUser&1) && !(beforeAnswer&1) && Boolean(afterUser&following) && Boolean(beforeAnswer&following);
      });
      // Multiple external galleries are ambiguous even if one has matching IDs.
      return galleries.length===1?[...scopes,galleries[0]]:scopes;
    }

    function key(node) {
      if (!node || excluded(node)) return '';
      const turn = owner(node) || node;
      const message = turn.matches?.(MESSAGE_SELECTOR) ? turn : turn.querySelector?.(MESSAGE_SELECTOR);
      // Indices, test IDs, text hashes and React node references are not
      // logical identities. Anonymous nodes remain diagnostic observations.
      return turn.getAttribute?.('data-turn-id')
        || turn.getAttribute?.('data-turn-id-container')
        || turn.getAttribute?.('data-message-id')
        || message?.getAttribute?.('data-message-id') || globalThis.ChatGptImageRecordProof?.key(turn) || '';
    }

    function credibleMessage(node) {
      return node?.matches?.('[data-message-author-role="assistant"]')
        && Boolean(node.getAttribute?.('data-message-id') || node.getAttribute?.('data-message-model-slug')
          || node.hasAttribute?.('data-turn-start-message') || node.matches?.('.markdown')
          || node.querySelector?.('.markdown, [data-start][data-end], pre, code'));
    }

    function getFinalAssistantNode(root) {
      if (!root || excluded(root) || role(root) === 'user') return null;
      if (credibleMessage(root)) return root;
      const canonical = Array.from(root.querySelectorAll?.('[data-message-author-role="assistant"]') || [])
        .find((node) => !excluded(node) && credibleMessage(node));
      if (canonical) return canonical;
      if (role(root) === 'assistant') return finalAnswerNode(root);
      return Array.from(root.querySelectorAll?.('h4.sr-only') || [])
        .map((heading) => heading.parentElement)
        .filter((node) => !excluded(node) && role(node) === 'assistant')
        .map(finalAnswerNode).find(Boolean) || null;
    }

    return Object.freeze({
      getAssistantNodes: () => getTurnNodes().filter((node) => role(node) === 'assistant'),
      getFinalAssistantNode,
      getTurnNodes,
      generatedImageScopes,
      key,
      role,
      owner,
      excluded,
    });
  }

  globalThis.ChatGptTurnDom = Object.freeze({ createTurnDom });
})();
