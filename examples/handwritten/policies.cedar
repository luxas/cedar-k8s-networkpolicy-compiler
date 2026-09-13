// A hand-written translation of examples/bookinfo/networkpolicy.yaml, written the
// way a person would rather than the way the compiler does -- the `principal is Pod`
// guard sits in the scope instead of the condition, and the clauses are flat.
//
// tests/handwritten.rs asserts that this and the compiler's output agree
// decision-for-decision over the whole bookinfo grid, which is a real cross-check:
// two independent encodings of the same NetworkPolicy have to reach the same answers.

// No policy has policyTypes: Egress, so nothing is isolated for egress.
permit (principal, action == Action::"egress", resource);

// `details` is isolated for ingress; everything else is not.
permit (principal, action == Action::"ingress", resource)
unless {
  resource is Pod &&
  resource.namespace == Namespace::"default" &&
  resource.labels.hasTag("app") &&
  resource.labels.getTag("app") == "details"
};

// default/details-allow-productpage, ingress rule 0.
permit (
  principal is Pod,
  action == Action::"ingress",
  resource is Pod
)
when {
  principal.namespace == Namespace::"default" &&
  principal.labels.hasTag("app") &&
  principal.labels.getTag("app") == "productpage" &&
  resource.namespace == Namespace::"default" &&
  resource.labels.hasTag("app") &&
  resource.labels.getTag("app") == "details" &&
  context.protocol == Protocol::"TCP" &&
  context.port == 9080
};
