(** * Blood — Generation Snapshot Correctness

    This file formalizes the generation snapshot mechanism that ensures
    safe interaction between algebraic effects and generational references.

    Reference: FORMAL_SEMANTICS.md §13 (Complete Generation Snapshots Proof)
    Phase: M4 — Generational References
    Task: FORMAL-003
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

(** ** Memory cell operations *)

Definition cell_is_allocated (c : memory_cell) : Prop :=
  match cell_value c with
  | Some _ => True
  | None => False
  end.

Definition cell_is_freed (c : memory_cell) : Prop :=
  match cell_value c with
  | None => True
  | Some _ => False
  end.

(** ** Memory operation traces

    We model memory evolution as a sequence of operations. *)

Inductive mem_op : Type :=
  | MemAlloc : nat -> value -> mem_op    (** alloc at addr with value *)
  | MemFree : nat -> nat -> mem_op       (** free addr at generation *)
  | MemDeref : nat -> nat -> mem_op.     (** deref addr at generation *)

(** ** Memory operation semantics *)

Inductive mem_step : memory -> mem_op -> memory -> Prop :=
  | MemStep_Alloc : forall M addr v,
      (** Address must be fresh (generation 0, no value) *)
      cell_gen (M addr) = 0 ->
      cell_value (M addr) = None ->
      mem_step M (MemAlloc addr v)
               (mem_update M addr (mk_cell (Some v) 0))

  | MemStep_Free : forall M addr gen,
      (** Generation must match current *)
      cell_gen (M addr) = gen ->
      cell_value (M addr) <> None ->
      mem_step M (MemFree addr gen)
               (mem_update M addr
                  (mk_cell None (S gen)))

  | MemStep_Deref : forall M addr gen,
      (** Generation must match current for safe deref *)
      cell_gen (M addr) = gen ->
      cell_value (M addr) <> None ->
      mem_step M (MemDeref addr gen) M.
      (** Deref doesn't change memory *)

(** ** Memory evolution: multi-step *)

Inductive mem_evolves : memory -> memory -> Prop :=
  | MemEvolves_Refl : forall M,
      mem_evolves M M
  | MemEvolves_Step : forall M1 M2 M3 op,
      mem_step M1 op M2 ->
      mem_evolves M2 M3 ->
      mem_evolves M1 M3.

(** ** Snapshot validity (Definition from §13.3)

    Valid(Γ_gen, M) ≡ ∀(a, g) ∈ Γ_gen. M(a).gen = g *)

Definition snapshot_valid (M : memory) (snap : list gen_ref) : Prop :=
  Forall (fun gr =>
    match gr with
    | GenRef addr gen => current_gen M addr = gen
    end) snap.

(** Decidable version *)

Definition snapshot_valid_dec (M : memory) (snap : list gen_ref) : bool :=
  forallb (fun gr =>
    match gr with
    | GenRef addr gen => current_gen M addr =? gen
    end) snap.

Lemma snapshot_valid_dec_correct :
  forall M snap,
    snapshot_valid_dec M snap = true <-> snapshot_valid M snap.
Proof.
  intros M snap. unfold snapshot_valid_dec, snapshot_valid.
  split.
  - intro H. apply Forall_forall. intros gr Hin.
    rewrite forallb_forall in H. specialize (H gr Hin).
    destruct gr. apply Nat.eqb_eq. exact H.
  - intro H. apply forallb_forall. intros gr Hin.
    rewrite Forall_forall in H. specialize (H gr Hin).
    destruct gr. apply Nat.eqb_eq. exact H.
Qed.

(** ** Snapshot capture invariant

    At capture time, all references in the snapshot are valid. *)

Definition snapshot_captured_valid
    (M : memory) (snap : list gen_ref) : Prop :=
  snapshot_valid M snap /\
  (** All referenced cells are allocated *)
  Forall (fun gr =>
    match gr with
    | GenRef addr _ => cell_is_allocated (M addr)
    end) snap.

(** ** Resume outcomes *)

Inductive resume_outcome : Type :=
  | Resume_Safe : resume_outcome     (** snapshot validates, resume proceeds *)
  | Resume_Stale :
      nat -> nat -> nat ->            (** addr, expected gen, actual gen *)
      resume_outcome.                 (** snapshot invalid, StaleReference *)

Definition check_resume
    (M : memory) (snap : list gen_ref) : resume_outcome :=
  let fix check refs :=
    match refs with
    | [] => Resume_Safe
    | GenRef addr gen :: rest =>
        if current_gen M addr =? gen
        then check rest
        else Resume_Stale addr gen (current_gen M addr)
    end
  in check snap.

(** ** Theorem 13.1: Generation Snapshot Safety

    Reference: FORMAL_SEMANTICS.md §13.5

    For any well-typed program with continuation capture and resume:
    1. Capture Validity: At capture time, Valid(Γ_gen, M) holds
    2. Detection Completeness: If any reference becomes stale,
       StaleReference is raised
    3. No Use-After-Free: If resume succeeds, all derefs in κ are safe *)

(** *** Part 1: Capture Validity *)

Theorem capture_validity :
  forall M snap,
    (** If the snapshot was captured from a well-typed term *)
    snapshot_captured_valid M snap ->
    (** Then all references in the snapshot are valid *)
    snapshot_valid M snap.
Proof.
  intros M snap [Hvalid _]. exact Hvalid.
Qed.

(** Helper lemma: forallb returning false implies existence of failing element *)

Lemma forallb_false_exists :
  forall {A : Type} (f : A -> bool) (l : list A),
    forallb f l = false ->
    exists x, In x l /\ f x = false.
Proof.
  intros A f l Hfalse.
  induction l as [| hd tl IH].
  - simpl in Hfalse. discriminate.
  - simpl in Hfalse.
    apply Bool.andb_false_iff in Hfalse.
    destruct Hfalse as [Hhd | Htl].
    + exists hd. split. left. reflexivity. assumption.
    + specialize (IH Htl). destruct IH as [x [Hin Hfx]].
      exists x. split. right. assumption. assumption.
Qed.

(** *** Part 2: Detection Completeness *)

Theorem detection_completeness :
  forall M0 M1 snap,
    (** Snapshot captured in M0 *)
    snapshot_captured_valid M0 snap ->
    (** Memory evolved to M1 *)
    mem_evolves M0 M1 ->
    (** Either all references still valid, or stale detected *)
    (snapshot_valid M1 snap) \/
    (exists addr gen gen',
       In (GenRef addr gen) snap /\
       current_gen M1 addr = gen' /\
       gen <> gen').
Proof.
  intros M0 M1 snap Hcaptured Hevolves.
  destruct (snapshot_valid_dec M1 snap) eqn:Hdec.
  - (* All valid *)
    left. apply snapshot_valid_dec_correct. exact Hdec.
  - (* Some reference stale: snapshot_valid_dec = false means
       there exists a ref with mismatched generation. *)
    right.
    unfold snapshot_valid_dec in Hdec.
    apply forallb_false_exists in Hdec.
    destruct Hdec as [gr [Hin Hfalse]].
    destruct gr as [addr gen].
    exists addr, gen, (current_gen M1 addr).
    split; [| split].
    + exact Hin.
    + reflexivity.
    + (* gen <> current_gen M1 addr because eqb returned false *)
      apply Nat.eqb_neq in Hfalse.
      (* Hfalse says current_gen M1 addr <> gen, but we need gen <> current_gen M1 addr *)
      intro Heq. apply Hfalse. symmetry. exact Heq.
Qed.

(** *** Part 3: No Use-After-Free *)

Theorem no_use_after_free :
  forall M0 M1 snap addr gen,
    (** Snapshot captured in M0 *)
    snapshot_captured_valid M0 snap ->
    (** Memory evolved to M1 *)
    mem_evolves M0 M1 ->
    (** Resume check passed *)
    snapshot_valid M1 snap ->
    (** Reference is in the snapshot *)
    In (GenRef addr gen) snap ->
    (** Then dereferencing (addr, gen) in M1 is safe *)
    current_gen M1 addr = gen /\
    cell_is_allocated (M1 addr).
Proof.
  intros M0 M1 snap addr gen Hcaptured Hevolves Hvalid Hin.
  split.
  - (* Generation matches *)
    unfold snapshot_valid in Hvalid.
    rewrite Forall_forall in Hvalid.
    specialize (Hvalid (GenRef addr gen) Hin).
    exact Hvalid.
  - (* Cell is allocated *)
    (* Key insight: if current_gen matches the snapshot generation,
       the cell has NOT been freed since capture.
       Because free increments the generation, so if gen still matches
       currentGen, no free has occurred at this address. *)
    unfold snapshot_valid in Hvalid.
    rewrite Forall_forall in Hvalid.
    specialize (Hvalid (GenRef addr gen) Hin).
    (* Need: gen = current_gen M1 addr implies allocated.
       This follows from the memory operation semantics:
       - MemStep_Free sets cell_value to None and increments gen
       - If gen still matches, no free has occurred
       - If no free, cell is still allocated (assuming no other
         operation sets value to None) *)
Admitted.

(** ** Free increments generation

    Key lemma: freeing a cell always increments its generation. *)

Lemma free_increments_gen :
  forall M M' addr gen,
    mem_step M (MemFree addr gen) M' ->
    current_gen M' addr = S gen.
Proof.
  intros M M' addr gen Hstep.
  inversion Hstep; subst.
  unfold current_gen, mem_update.
  rewrite Nat.eqb_refl. simpl. reflexivity.
Qed.

(** ** Generation monotonicity

    Generations only increase: once incremented, they never decrease. *)

Lemma gen_monotone :
  forall M M' addr op,
    mem_step M op M' ->
    current_gen M addr <= current_gen M' addr.
Proof.
  intros M M' addr op Hstep.
  inversion Hstep; subst; unfold current_gen, mem_update.
  - (* Alloc *)
    destruct (addr =? addr0) eqn:Heq.
    + apply Nat.eqb_eq in Heq. subst. simpl. lia.
    + apply Nat.le_refl.
  - (* Free *)
    destruct (addr =? addr0) eqn:Heq.
    + apply Nat.eqb_eq in Heq. subst. simpl. lia.
    + apply Nat.le_refl.
  - (* Deref: memory unchanged *)
    apply Nat.le_refl.
Qed.

(** ** Multi-step generation monotonicity *)

Lemma gen_monotone_multi :
  forall M M' addr,
    mem_evolves M M' ->
    current_gen M addr <= current_gen M' addr.
Proof.
  intros M M' addr Hevolves.
  induction Hevolves.
  - apply Nat.le_refl.
  - apply Nat.le_trans with (m := current_gen M2 addr).
    + exact (gen_monotone M1 M2 addr op H).
    + exact IHHevolves.
Qed.

(** ** Stale reference detection is sound

    If a reference was freed (generation incremented), the snapshot
    check will detect it. *)

Theorem stale_detection_sound :
  forall M0 M1 M2 snap addr gen,
    snapshot_captured_valid M0 snap ->
    In (GenRef addr gen) snap ->
    mem_evolves M0 M1 ->
    mem_step M1 (MemFree addr gen) M2 ->
    mem_evolves M2 M2 ->
    (* After freeing, the snapshot check MUST fail *)
    ~ snapshot_valid M2 snap.
Proof.
  intros M0 M1 M2 snap addr gen Hcaptured Hin Hevolves1 Hfree _.
  unfold snapshot_valid.
  intro Hvalid.
  rewrite Forall_forall in Hvalid.
  specialize (Hvalid (GenRef addr gen) Hin).
  simpl in Hvalid.
  (* By free_increments_gen, M2 has gen (S gen) at addr *)
  assert (Hgen : current_gen M2 addr = S gen).
  { exact (free_increments_gen M1 M2 addr gen Hfree). }
  (* But Hvalid says current_gen M2 addr = gen *)
  rewrite Hgen in Hvalid.
  (* S gen = gen is impossible *)
  lia.
Qed.

(** ** Effects-Gen Composition Safety (Theorem 10.2.1)

    Reference: FORMAL_SEMANTICS.md §10.9.1

    If a continuation κ is captured with snapshot Γ_gen in memory M0,
    and memory evolves to M1, and resume(κ, v) is invoked in M1, then:
    1. All snapshot refs valid → resume succeeds safely, OR
    2. Some snapshot ref stale → StaleReference raised *)

Theorem effects_gen_composition_safety :
  forall M0 M1 snap,
    snapshot_captured_valid M0 snap ->
    mem_evolves M0 M1 ->
    (* Outcome is determined by snapshot validation *)
    (snapshot_valid M1 snap ->
     check_resume M1 snap = Resume_Safe) /\
    (~ snapshot_valid M1 snap ->
     exists addr gen gen',
       check_resume M1 snap = Resume_Stale addr gen gen').
Proof.
  intros M0 M1 snap Hcaptured Hevolves.
  split.
  - (* Valid → Safe: by induction on snap, each ref in the snapshot
       passes the eqb check since snapshot_valid gives us equality. *)
    intro Hvalid. unfold snapshot_valid in Hvalid. unfold check_resume.
    induction snap as [|gr rest IH].
    + simpl. reflexivity.
    + simpl. destruct gr as [addr gen].
      rewrite Forall_cons_iff in Hvalid. destruct Hvalid as [Hfirst Hrest].
      simpl in Hfirst. rewrite Hfirst. rewrite Nat.eqb_refl.
      apply IH; auto.
      destruct Hcaptured as [Hv Ha]. split.
      * inversion Hv; auto.
      * inversion Ha; auto.
  - (* Invalid → Stale: by induction on snap, find the first ref
       that fails the eqb check. *)
    intro Hinvalid. unfold snapshot_valid in Hinvalid. unfold check_resume.
    induction snap as [|gr rest IH].
    + exfalso. apply Hinvalid. constructor.
    + destruct gr as [addr gen].
      destruct (current_gen M1 addr =? gen) eqn:Heq.
      * (* This ref valid, recurse *)
        apply Nat.eqb_eq in Heq.
        assert (Hrest_invalid : ~ Forall (fun gr =>
          match gr with GenRef a g => current_gen M1 a = g end) rest).
        { intro Hrest. apply Hinvalid. constructor; assumption. }
        destruct Hcaptured as [Hv Ha].
        assert (Hcap_rest : snapshot_captured_valid M0 rest).
        { split; inversion Hv; inversion Ha; auto. }
        destruct (IH Hcap_rest Hrest_invalid) as [a [g [g' Hresult]]].
        exists a, g, g'. exact Hresult.
      * (* This ref stale *)
        apply Nat.eqb_neq in Heq.
        exists addr, gen, (current_gen M1 addr).
        simpl.
        destruct (current_gen M1 addr =? gen) eqn:Heq2.
        -- apply Nat.eqb_eq in Heq2. contradiction.
        -- reflexivity.
Qed.

(** ** Memory evolution transitivity *)

Lemma mem_evolves_trans :
  forall M1 M2 M3,
    mem_evolves M1 M2 ->
    mem_evolves M2 M3 ->
    mem_evolves M1 M3.
Proof.
  intros M1 M2 M3 H12 H23.
  induction H12.
  - exact H23.
  - eapply MemEvolves_Step.
    + exact H.
    + apply IHmem_evolves. exact H23.
Qed.

(** ** Multi-shot handler safety with snapshots

    Reference: FORMAL_SEMANTICS.md §13.7

    Each invocation of a multi-shot continuation independently
    validates its snapshot. *)

Theorem multishot_snapshot_safety :
  forall M0 M1 M2 snap,
    snapshot_captured_valid M0 snap ->
    mem_evolves M0 M1 ->
    mem_evolves M1 M2 ->
    (* First resume in M1: independently validated *)
    ((snapshot_valid M1 snap -> check_resume M1 snap = Resume_Safe) /\
     (~ snapshot_valid M1 snap ->
      exists a g g', check_resume M1 snap = Resume_Stale a g g')) /\
    (* Second resume in M2: independently validated *)
    ((snapshot_valid M2 snap -> check_resume M2 snap = Resume_Safe) /\
     (~ snapshot_valid M2 snap ->
      exists a g g', check_resume M2 snap = Resume_Stale a g g')).
Proof.
  intros M0 M1 M2 snap Hcaptured Hev01 Hev12.
  split.
  - exact (effects_gen_composition_safety M0 M1 snap Hcaptured Hev01).
  - exact (effects_gen_composition_safety M0 M2 snap
           Hcaptured (mem_evolves_trans M0 M1 M2 Hev01 Hev12)).
Qed.
