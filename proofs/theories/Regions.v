(** * Blood — Region Safety via Generational References

    Formalizes the safety of Blood's region-based memory allocation.
    Region destruction bumps generations for all region-allocated addresses,
    invalidating any outstanding references. Generation snapshots detect
    these invalidated references at resume time.

    Reference: FORMAL_SEMANTICS.md §5.8, MEMORY_MODEL.md §7
    Phase: Phase 5 — Tier 2 (Regions x Generations)

    Key insight (FORMAL_SEMANTICS.md §5.8): "Region safety is NOT a
    typing property — it is a runtime property guaranteed by the
    generation system." The typing rule T_Region is a trivial pass-through.
    All safety comes from generation bumps on region destruction.

    Status: 0 Admitted.
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
From Stdlib Require Import PeanoNat.
From Stdlib Require Import Lia.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.
From Blood Require Import Substitution.
From Blood Require Import Semantics.
From Blood Require Import GenerationSnapshots.

(** ** Region definition

    A region is a list of memory addresses allocated within it.
    Region destruction frees all addresses, bumping their generations. *)

Definition region := list nat.

(** ** Region destruction

    Frees each address in the region by clearing its value and
    incrementing its generation. This is the bulk-free operation
    from MEMORY_MODEL.md §7.5. *)

Fixpoint region_destroy (r : region) (M : memory) : memory :=
  match r with
  | [] => M
  | addr :: rest =>
      let old_cell := M addr in
      let new_cell := mk_cell None (S (cell_gen old_cell)) in
      region_destroy rest (mem_update M addr new_cell)
  end.

(** ** Region well-formedness

    A region is well-formed in memory M if all its addresses are
    currently allocated (have Some value). *)

Definition region_well_formed (r : region) (M : memory) : Prop :=
  Forall (fun addr => cell_is_allocated (M addr)) r.

(** ** No duplicate addresses in a region *)

Definition region_no_dup (r : region) : Prop :=
  NoDup r.

(** ** Address membership *)

Definition addr_in_region (addr : nat) (r : region) : Prop :=
  In addr r.

(** ** Disjoint regions *)

Definition region_disjoint (r1 r2 : region) : Prop :=
  forall addr, ~ (In addr r1 /\ In addr r2).

(** ** Key lemma: mem_update at addr does not affect other addresses *)

Lemma mem_update_other :
  forall M addr addr' c,
    addr <> addr' ->
    mem_update M addr c addr' = M addr'.
Proof.
  intros M addr addr' c Hneq.
  unfold mem_update.
  destruct (addr' =? addr) eqn:Heq.
  - apply Nat.eqb_eq in Heq. symmetry in Heq. contradiction.
  - reflexivity.
Qed.

Lemma mem_update_same :
  forall M addr c,
    mem_update M addr c addr = c.
Proof.
  intros M addr c.
  unfold mem_update.
  rewrite Nat.eqb_refl. reflexivity.
Qed.

(** ** Region destruction preserves addresses outside the region *)

Lemma region_destroy_preserves_other :
  forall r M addr,
    ~ In addr r ->
    region_destroy r M addr = M addr.
Proof.
  intros r. induction r as [| a rest IH]; intros M addr Hnotin.
  - simpl. reflexivity.
  - simpl.
    assert (Ha_ne : a <> addr).
    { intro Heq. apply Hnotin. left. exact Heq. }
    assert (Hnotin_rest : ~ In addr rest).
    { intro Hin. apply Hnotin. right. exact Hin. }
    rewrite IH; auto.
    apply mem_update_other. exact Ha_ne.
Qed.

(** ** Region destruction bumps generation at each address *)

Lemma region_destroy_bumps_gen :
  forall r M addr,
    In addr r ->
    NoDup r ->
    current_gen (region_destroy r M) addr =
      S (current_gen M addr).
Proof.
  intros r. induction r as [| a rest IH]; intros M addr Hin Hnd.
  - inversion Hin.
  - inversion Hnd as [| ? ? Hnotin Hnd_rest]. subst.
    simpl. destruct Hin as [Heq | Hin_rest].
    + (* addr = a: mem_update sets gen to S gen, rest doesn't touch addr *)
      subst a.
      unfold current_gen.
      rewrite region_destroy_preserves_other; auto.
      rewrite mem_update_same. simpl. reflexivity.
    + (* addr in rest: mem_update at a doesn't touch addr *)
      assert (Ha_ne : a <> addr).
      { intro Heq. subst a. contradiction. }
      rewrite IH; auto.
      unfold current_gen. rewrite mem_update_other; auto.
Qed.

(** Corollary: generation preserved outside region *)

Lemma region_destroy_gen_preserved :
  forall r M addr,
    ~ In addr r ->
    current_gen (region_destroy r M) addr = current_gen M addr.
Proof.
  intros r M addr Hnotin.
  unfold current_gen. rewrite region_destroy_preserves_other; auto.
Qed.

(** ** Theorem 1: Region Safety

    After region destruction, any generation snapshot that contains
    a reference to a region-allocated address becomes invalid.
    The address's generation was bumped, so the snapshot's recorded
    generation no longer matches.

    Reference: FORMAL_SEMANTICS.md §5.8.2 [Region-Invalidation] *)

Theorem region_safety :
  forall r M snap,
    NoDup r ->
    (** Snapshot was valid before region destruction *)
    snapshot_valid M snap ->
    (** Snapshot contains at least one reference to a region address *)
    (exists addr gen,
       In (GenRef addr gen) snap /\ In addr r) ->
    (** After destruction, snapshot is invalid *)
    ~ snapshot_valid (region_destroy r M) snap.
Proof.
  intros r M snap Hnd Hvalid [addr [gen [Hin_snap Hin_r]]] Hvalid'.
  (* By snapshot_valid before destruction, gen = current_gen M addr *)
  unfold snapshot_valid in Hvalid.
  rewrite Forall_forall in Hvalid.
  specialize (Hvalid (GenRef addr gen) Hin_snap). simpl in Hvalid.
  (* By snapshot_valid after destruction, gen = current_gen (region_destroy r M) addr *)
  unfold snapshot_valid in Hvalid'.
  rewrite Forall_forall in Hvalid'.
  specialize (Hvalid' (GenRef addr gen) Hin_snap). simpl in Hvalid'.
  (* But region_destroy bumps the generation *)
  rewrite region_destroy_bumps_gen in Hvalid'; auto.
  (* So S (current_gen M addr) = gen, and current_gen M addr = gen *)
  rewrite Hvalid in Hvalid'.
  (* S gen = gen — impossible *)
  lia.
Qed.

(** ** Theorem 2: Region Effect Safety

    Connects region destruction to the effect-generation composition
    from GenerationSnapshots.v. If a continuation's snapshot references
    a region-allocated address and the region is destroyed before resume,
    the snapshot check detects the staleness.

    This is a corollary of region_safety combined with
    effects_gen_composition_safety.

    Reference: FORMAL_SEMANTICS.md §5.8.3 *)

Theorem region_effect_safety :
  forall r M0 M1 snap,
    NoDup r ->
    (** Snapshot captured in M0 *)
    snapshot_captured_valid M0 snap ->
    (** Memory evolved from M0 to M1 *)
    mem_evolves M0 M1 ->
    (** Snapshot contains a region address that is still valid in M1 *)
    (exists addr gen,
       In (GenRef addr gen) snap /\
       In addr r /\
       current_gen M1 addr = gen) ->
    (** After region destruction in M1, check_resume detects staleness *)
    exists addr' gen' gen'',
      check_resume (region_destroy r M1) snap =
        Resume_Stale addr' gen' gen''.
Proof.
  intros r M0 M1 snap Hnd Hcaptured Hevolves
    [addr [gen [Hin_snap [Hin_r Hgen]]]].
  (* After region_destroy, addr has generation S gen *)
  assert (Hbumped : current_gen (region_destroy r M1) addr = S gen).
  { rewrite (region_destroy_bumps_gen r M1 addr Hin_r Hnd).
    rewrite Hgen. reflexivity. }
  (* Prove by induction on snap that check_resume returns Resume_Stale.
     The snapshot records gen for addr, but memory now has S gen,
     so check_resume must find this mismatch. *)
  (* Prove check_resume returns Resume_Stale by finding the mismatched ref.
     We generalize over snap and Hin_snap to allow induction. *)
  revert Hcaptured.
  induction snap as [| gr rest IH]; intros Hcaptured.
  - inversion Hin_snap.
  - destruct gr as [a g]. simpl.
    destruct (current_gen (region_destroy r M1) a =? g) eqn:Heq.
    + (* This ref passes — check rest *)
      destruct Hin_snap as [Hfirst | Hrest].
      * (* addr = a, gen = g — but the check passed? contradiction *)
        inversion Hfirst. subst a g.
        apply Nat.eqb_eq in Heq.
        rewrite Hbumped in Heq. lia.
      * (* Need snapshot_captured_valid M0 rest *)
        apply IH; auto.
        destruct Hcaptured as [Hv Ha].
        split; inversion Hv; inversion Ha; auto.
    + (* This ref fails — found stale *)
      exists a, g, (current_gen (region_destroy r M1) a).
      reflexivity.
Qed.

(** ** Theorem 3: Escape Analysis Soundness

    A reference to a region-allocated address is guaranteed stale
    after region destruction. The generation is strictly incremented,
    so any dereference will fail the generation check.

    Reference: FORMAL_SEMANTICS.md §5.8.2 [Region-Stale-Detect] *)

Theorem escape_analysis_sound :
  forall r M addr,
    NoDup r ->
    In addr r ->
    (** Generation is strictly incremented *)
    current_gen (region_destroy r M) addr =
      S (current_gen M addr).
Proof.
  intros r M addr Hnd Hin.
  exact (region_destroy_bumps_gen r M addr Hin Hnd).
Qed.

(** ** Nested region safety

    Inner region destruction does not affect outer region addresses.
    This follows from region_destroy_preserves_other and disjointness. *)

Theorem region_nested_safety :
  forall r_inner r_outer M addr,
    region_disjoint r_inner r_outer ->
    In addr r_outer ->
    (** Inner region destruction preserves outer references *)
    current_gen (region_destroy r_inner M) addr =
      current_gen M addr.
Proof.
  intros r_inner r_outer M addr Hdisj Hin_outer.
  apply region_destroy_gen_preserved.
  intro Hin_inner.
  exact (Hdisj addr (conj Hin_inner Hin_outer)).
Qed.

(** ** Region destruction is deterministic *)

Lemma region_destroy_deterministic :
  forall r M,
    exists M', region_destroy r M = M'.
Proof.
  intros r M. exists (region_destroy r M). reflexivity.
Qed.

(** ** Summary

    Phase 5 establishes the following:

    1. region_safety: Region destruction invalidates snapshots
       referencing region-allocated addresses.
    2. region_effect_safety: check_resume detects region-killed
       references (returns Resume_Stale).
    3. escape_analysis_sound: Generation strictly incremented
       at each region address after destruction.
    4. region_nested_safety: Inner region destruction does not
       affect outer region references (via disjointness).

    All theorems build on GenerationSnapshots.v infrastructure.
    No modifications to existing files.

    Status: 0 Admitted, 0 Axioms, 0 Parameters.
*)
